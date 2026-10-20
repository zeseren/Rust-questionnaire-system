//! 文本文法交给 pest；先限制解析资源，再绑定身份并复用统一 AST 与语义编译。

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::LazyLock,
};

use pest::{
    Parser,
    error::InputLocation,
    iterators::{Pair, Pairs},
    pratt_parser::{Assoc, Op, PrattParser},
};
use serde::Serialize;
use serde_json::{Number, Value, json};
use uuid::Uuid;

use crate::domain::{Question, QuestionKind};
use crate::flow::{FlowBranch, FlowDefinition, FlowNode, FlowOutcome};
use crate::logic::{LogicError, LogicErrorCode, LogicValueType, check_raw_budget, error};
use crate::logic_document::{ComputedVariable, DocumentLogic, QuestionRule};

#[derive(pest_derive::Parser)]
#[grammar = "src/logic_text.pest"]
struct Grammar;

static PRECEDENCE: LazyLock<PrattParser<Rule>> = LazyLock::new(|| {
    use Rule::*;
    PrattParser::new()
        .op(Op::infix(logical_or, Assoc::Left))
        .op(Op::infix(logical_and, Assoc::Left))
        .op(Op::infix(equal, Assoc::Left)
            | Op::infix(not_equal, Assoc::Left)
            | Op::infix(less, Assoc::Left)
            | Op::infix(less_equal, Assoc::Left)
            | Op::infix(greater, Assoc::Left)
            | Op::infix(greater_equal, Assoc::Left))
        .op(Op::infix(add, Assoc::Left) | Op::infix(subtract, Assoc::Left))
        .op(Op::infix(multiply, Assoc::Left) | Op::infix(divide, Assoc::Left))
        .op(Op::prefix(negation))
});

/// 文本错误携带源位置，语义错误同时保留原 AST 路径。
#[derive(Debug, thiserror::Error, Serialize)]
#[error("{error}（第 {line} 行，第 {column} 列）")]
pub struct LogicTextError {
    /// 稳定分类、题目身份和 AST 路径。
    #[serde(flatten)]
    #[source]
    pub error: LogicError,
    /// 从 1 开始的行号。
    pub line: usize,
    /// 从 1 开始的 Unicode 标量列号。
    pub column: usize,
}

fn located(source: &str, offset: usize, error: LogicError) -> LogicTextError {
    // 偏移来自 pest 的字节位置；越界或非字符边界时退化为整段源码，避免切片 panic。
    let prefix = source.get(..offset).unwrap_or(source);
    LogicTextError {
        line: prefix.bytes().filter(|byte| *byte == b'\n').count() + 1,
        column: prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1,
        error,
    }
}

fn failure(source: &str, offset: usize, code: LogicErrorCode) -> LogicTextError {
    located(source, offset, error(code, "$", None))
}

fn parse(source: &str, rule: Rule) -> Result<Pairs<'_, Rule>, LogicTextError> {
    Grammar::parse(rule, source).map_err(|error| {
        let offset = match error.location {
            InputLocation::Pos(offset) | InputLocation::Span((offset, _)) => offset,
        };
        // pest 的原始诊断包含输入片段；对外只保留位置与统一分类。
        failure(source, offset, LogicErrorCode::InvalidSyntax)
    })
}

fn next<'a>(pairs: &mut Pairs<'a, Rule>) -> Pair<'a, Rule> {
    pairs.next().expect("匹配成功的文法必须包含此字段")
}

fn string(source: &str, pair: &Pair<'_, Rule>) -> Result<String, LogicTextError> {
    let fail = || {
        failure(
            source,
            pair.as_span().start(),
            LogicErrorCode::InvalidSyntax,
        )
    };
    serde_json::from_str(pair.as_str()).map_err(|_| fail())
}

fn check_text_budget(source: &str) -> Result<(), LogicTextError> {
    if source.len() > 1_048_576 {
        return Err(failure(source, 0, LogicErrorCode::LimitExceeded));
    }
    let lexicon = next(&mut parse(source, Rule::lexicon)?);
    // 每层记录尚未结束的前缀运算；括号、函数与 ! 共用递归深度预算。
    let mut prefixes = vec![0_usize];
    let mut depth = 1;
    for token in lexicon.into_inner() {
        if token.as_rule() == Rule::EOI {
            break;
        }
        let offset = token.as_span().start();
        if token.as_rule() == Rule::string {
            string(source, &token)?;
            continue;
        }
        if token.as_rule() != Rule::symbol {
            continue;
        }
        match token.as_str() {
            "!" => {
                *prefixes.last_mut().expect("始终保留表达式外层") += 1;
                depth += 1;
            }
            "(" => {
                prefixes.push(0);
                depth += 1;
            }
            ")" if prefixes.len() > 1 => {
                depth -= 1 + prefixes.pop().expect("已检查存在内层");
            }
            "&&" | "||" | "==" | "!=" | "<" | "<=" | ">" | ">=" | "+" | "-" | "*" | "/" | ","
            | ";" | "->" | "}" => {
                depth -= std::mem::take(prefixes.last_mut().expect("始终保留表达式外层"));
            }
            _ => {}
        }
        if depth > 64 {
            return Err(failure(source, offset, LogicErrorCode::LimitExceeded));
        }
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Category {
    Question,
    Option,
    Variable,
    Node,
}

#[derive(Clone, Copy)]
struct Binding {
    category: Category,
    id: Uuid,
}

struct Compiler<'a> {
    source: &'a str,
    bindings: BTreeMap<&'a str, Binding>,
    locations: Vec<(String, usize)>,
}

impl<'a> Compiler<'a> {
    fn failure(&self, pair: &Pair<'_, Rule>, code: LogicErrorCode) -> LogicTextError {
        failure(self.source, pair.as_span().start(), code)
    }

    fn atom(&self, pair: Pair<'a, Rule>) -> Result<Pair<'a, Rule>, LogicTextError> {
        if pair.as_rule() != Rule::expression {
            return Err(self.failure(&pair, LogicErrorCode::InvalidSyntax));
        }
        let mut parts = pair.into_inner();
        let atom = next(&mut parts);
        if let Some(extra) = parts.next() {
            return Err(self.failure(&extra, LogicErrorCode::InvalidSyntax));
        }
        Ok(atom)
    }

    fn reference(&self, pair: Pair<'a, Rule>, category: Category) -> Result<Uuid, LogicTextError> {
        let pair = if pair.as_rule() == Rule::expression {
            self.atom(pair)?
        } else {
            pair
        };
        // 引用函数只接受显式别名；true / false 在表达式位置是布尔常量。
        if pair.as_rule() != Rule::identifier {
            return Err(self.failure(&pair, LogicErrorCode::InvalidSyntax));
        }
        self.bindings
            .get(pair.as_str())
            .filter(|binding| binding.category == category)
            .map(|binding| binding.id)
            .ok_or_else(|| self.failure(&pair, LogicErrorCode::InvalidReference))
    }

    fn references(
        &self,
        pair: Pair<'a, Rule>,
        category: Category,
    ) -> Result<Vec<Uuid>, LogicTextError> {
        if pair.as_rule() != Rule::references {
            return Err(self.failure(&pair, LogicErrorCode::InvalidSyntax));
        }
        pair.into_inner()
            .map(|pair| self.reference(pair, category))
            .collect()
    }

    fn checked(&self, value: Value, offset: usize) -> Result<Value, LogicTextError> {
        // 每次构造即检查，长左结合链不会先形成超深 AST 再等待最终验证。
        check_raw_budget(Some(&value), 1, &mut 0)
            .map_err(|error| located(self.source, offset, error))?;
        Ok(value)
    }

    fn expression(&self, pair: Pair<'a, Rule>) -> Result<Value, LogicTextError> {
        if pair.as_rule() != Rule::expression {
            return Err(self.failure(&pair, LogicErrorCode::InvalidSyntax));
        }
        PRECEDENCE
            .map_primary(|pair| self.primary(pair))
            .map_prefix(|op, value| {
                self.checked(
                    json!({"op":"not", "expression":value?}),
                    op.as_span().start(),
                )
            })
            .map_infix(|left, op, right| {
                let left = left?;
                let right = right?;
                let value = match op.as_rule() {
                    Rule::logical_or | Rule::logical_and => {
                        let name = if op.as_rule() == Rule::logical_or {
                            "any"
                        } else {
                            "all"
                        };
                        json!({"op":name, "expressions":[left,right]})
                    }
                    Rule::equal
                    | Rule::not_equal
                    | Rule::less
                    | Rule::less_equal
                    | Rule::greater
                    | Rule::greater_equal => {
                        let comparison = match op.as_rule() {
                            Rule::equal => "eq",
                            Rule::not_equal => "ne",
                            Rule::less => "lt",
                            Rule::less_equal => "le",
                            Rule::greater => "gt",
                            _ => "ge",
                        };
                        json!({"op":"compare", "comparison":comparison, "left":left, "right":right})
                    }
                    _ => {
                        let operator = match op.as_rule() {
                            Rule::add => "add",
                            Rule::subtract => "subtract",
                            Rule::multiply => "multiply",
                            _ => "divide",
                        };
                        json!({"op":"arithmetic", "operator":operator, "left":left, "right":right})
                    }
                };
                self.checked(value, op.as_span().start())
            })
            .parse(pair.into_inner())
    }

    fn primary(&self, pair: Pair<'a, Rule>) -> Result<Value, LogicTextError> {
        let offset = pair.as_span().start();
        let value = match pair.as_rule() {
            Rule::number | Rule::negative_number => {
                json!({"op":"number", "value":pair.as_str()})
            }
            Rule::string => json!({"op":"text", "value":string(self.source, &pair)?}),
            Rule::boolean => json!({"op":"boolean", "value":pair.as_str() == "true"}),
            Rule::identifier => {
                json!({"op":"variable", "variable_id":self.reference(pair, Category::Variable)?})
            }
            Rule::group => return self.expression(next(&mut pair.into_inner())),
            Rule::call => return self.call(pair),
            _ => return Err(self.failure(&pair, LogicErrorCode::InvalidSyntax)),
        };
        self.checked(value, offset)
    }

    fn call(&self, pair: Pair<'a, Rule>) -> Result<Value, LogicTextError> {
        let offset = pair.as_span().start();
        let end = pair.as_span().end() - 1;
        let mut parts = pair.into_inner();
        let name = next(&mut parts);
        let op = name.as_str();
        let arguments: Vec<_> = parts.collect();
        let arity = match op {
            "answer" | "answered" | "variable" | "number" | "length" => Some(1),
            "selected" | "contains_any" | "contains_all" | "coalesce" => Some(2),
            "all" | "any" => None,
            _ => return Err(self.failure(&name, LogicErrorCode::InvalidSyntax)),
        };
        if arity.is_some_and(|arity| arguments.len() != arity) {
            return Err(failure(self.source, end, LogicErrorCode::InvalidSyntax));
        }
        let argument = |index: usize| arguments[index].clone();
        let value = match op {
            "answer" | "answered" => {
                json!({"op":op,"question_id":self.reference(argument(0), Category::Question)?})
            }
            "variable" => {
                json!({"op":op,"variable_id":self.reference(argument(0), Category::Variable)?})
            }
            "number" => {
                let text = self.atom(argument(0))?;
                if text.as_rule() != Rule::string {
                    return Err(self.failure(&text, LogicErrorCode::InvalidSyntax));
                }
                json!({"op":op,"value":string(self.source, &text)?})
            }
            "selected" => json!({
                "op":op,
                "question_id":self.reference(argument(0), Category::Question)?,
                "option_id":self.reference(argument(1), Category::Option)?
            }),
            "contains_any" | "contains_all" => json!({
                "op":op,
                "question_id":self.reference(argument(0), Category::Question)?,
                "option_ids":self.references(argument(1), Category::Option)?
            }),
            "all" | "any" => {
                let mut expressions = Vec::new();
                // 逐个参数在深度 2 处检查预算，整棵子树不再重复遍历。
                let mut count = 1;
                for argument in arguments {
                    let value = self.expression(argument)?;
                    check_raw_budget(Some(&value), 2, &mut count)
                        .map_err(|error| located(self.source, offset, error))?;
                    expressions.push(value);
                }
                return Ok(json!({"op":op,"expressions":expressions}));
            }
            "coalesce" => {
                json!({"op":op,"value":self.expression(argument(0))?,"fallback":self.expression(argument(1))?})
            }
            _ => json!({"op":op,"value":self.expression(argument(0))?}),
        };
        self.checked(value, offset)
    }

    fn compile(
        mut self,
        program: Pair<'a, Rule>,
        questions: &[Question],
    ) -> Result<DocumentLogic, LogicTextError> {
        let mut declarations = program.into_inner();
        let header = next(&mut declarations);
        let first_offset = header.as_span().start();
        let mut header = header.into_inner();
        next(&mut header);
        let dsl = next(&mut header);
        next(&mut header);
        let semantics = next(&mut header);
        let mut logic = DocumentLogic {
            dsl_version: dsl
                .as_str()
                .parse()
                .map_err(|_| self.failure(&dsl, LogicErrorCode::InvalidSyntax))?,
            semantics_version: semantics
                .as_str()
                .parse()
                .map_err(|_| self.failure(&semantics, LogicErrorCode::InvalidSyntax))?,
            variables: Vec::new(),
            rules: Vec::new(),
            flow: None,
        };
        let mut bound_ids = BTreeSet::new();
        let mut definitions = BTreeSet::new();
        let mut nodes = Vec::new();
        let mut entry = None;
        for declaration in declarations {
            let kind = declaration.as_rule();
            let offset = declaration.as_span().start();
            let mut fields = declaration.into_inner();
            match kind {
                Rule::binding => {
                    next(&mut fields);
                    let category = next(&mut fields);
                    let location = category.as_span().start();
                    let category = match category.as_str() {
                        "question" => Category::Question,
                        "option" => Category::Option,
                        "variable" => Category::Variable,
                        _ => Category::Node,
                    };
                    let alias = next(&mut fields);
                    let raw = string(self.source, &next(&mut fields))?;
                    let fail = || failure(self.source, location, LogicErrorCode::InvalidReference);
                    let id = crate::identity::parse(&raw).ok_or_else(fail)?;
                    if self.bindings.contains_key(alias.as_str()) || !bound_ids.insert(id) {
                        return Err(fail());
                    }
                    self.bindings
                        .insert(alias.as_str(), Binding { category, id });
                }
                Rule::variable_definition => {
                    next(&mut fields);
                    let id = self.reference(next(&mut fields), Category::Variable)?;
                    if !definitions.insert(id) {
                        return Err(failure(
                            self.source,
                            offset,
                            LogicErrorCode::InvalidReference,
                        ));
                    }
                    let value_type = LogicValueType::from_name(next(&mut fields).as_str())
                        .ok_or_else(|| {
                            failure(self.source, offset, LogicErrorCode::InvalidSyntax)
                        })?;
                    self.locations.push((
                        format!("$.logic.variables[{}]", logic.variables.len()),
                        offset,
                    ));
                    logic.variables.push(ComputedVariable {
                        id,
                        value_type,
                        expression: self.expression(next(&mut fields))?,
                    });
                }
                Rule::question_rule => {
                    let rule_kind = next(&mut fields);
                    let question_id = self.reference(next(&mut fields), Category::Question)?;
                    let index = if let Some(index) = logic
                        .rules
                        .iter()
                        .position(|rule| rule.question_id == question_id)
                    {
                        index
                    } else {
                        logic.rules.push(QuestionRule {
                            question_id,
                            visible_if: None,
                            required_if: None,
                            validate: None,
                        });
                        logic.rules.len() - 1
                    };
                    let (slot, key) = match rule_kind.as_str() {
                        "visible" => (&mut logic.rules[index].visible_if, "visible_if"),
                        "required" => (&mut logic.rules[index].required_if, "required_if"),
                        _ => (&mut logic.rules[index].validate, "validate"),
                    };
                    if slot.is_some() {
                        return Err(failure(self.source, offset, LogicErrorCode::InvalidSyntax));
                    }
                    self.locations
                        .push((format!("$.logic.rules[{index}].{key}"), offset));
                    *slot = Some(self.expression(next(&mut fields))?);
                }
                Rule::entry => {
                    if entry.is_some() {
                        return Err(failure(self.source, offset, LogicErrorCode::InvalidSyntax));
                    }
                    next(&mut fields);
                    entry = Some(self.reference(next(&mut fields), Category::Node)?);
                    self.locations.push(("$.logic.flow".into(), offset));
                }
                Rule::page | Rule::branch | Rule::end => {
                    next(&mut fields);
                    let id = self.reference(next(&mut fields), Category::Node)?;
                    if !definitions.insert(id) {
                        return Err(failure(
                            self.source,
                            offset,
                            LogicErrorCode::InvalidReference,
                        ));
                    }
                    self.locations
                        .push((format!("$.logic.flow.nodes[{}]", nodes.len()), offset));
                    let node = match kind {
                        Rule::page => FlowNode::Page {
                            id,
                            question_ids: self.references(next(&mut fields), Category::Question)?,
                            next: self.reference(next(&mut fields), Category::Node)?,
                        },
                        Rule::end => FlowNode::End {
                            id,
                            outcome: if next(&mut fields).as_str() == "complete" {
                                FlowOutcome::Complete
                            } else {
                                FlowOutcome::ScreenOut
                            },
                        },
                        _ => {
                            let mut branches = Vec::new();
                            let otherwise = loop {
                                let clause = next(&mut fields);
                                let kind = clause.as_rule();
                                let mut clause = clause.into_inner();
                                next(&mut clause);
                                if kind == Rule::otherwise {
                                    break self.reference(next(&mut clause), Category::Node)?;
                                }
                                branches.push(FlowBranch {
                                    when: self.expression(next(&mut clause))?,
                                    target: self.reference(next(&mut clause), Category::Node)?,
                                });
                            };
                            FlowNode::Branch {
                                id,
                                branches,
                                otherwise,
                            }
                        }
                    };
                    nodes.push(node);
                }
                Rule::EOI => {}
                _ => unreachable!("声明必须来自已匹配的文法"),
            }
        }
        if !nodes.is_empty() || entry.is_some() {
            let entry = entry.ok_or_else(|| {
                failure(self.source, self.source.len(), LogicErrorCode::InvalidFlow)
            })?;
            logic.flow = Some(FlowDefinition { entry, nodes });
        }
        // 预先索引题目与选项身份，绑定校验从逐项扫描降为集合查找。
        let mut question_ids = BTreeSet::new();
        let mut option_ids = BTreeSet::new();
        for question in questions {
            question_ids.insert(question.id);
            if let QuestionKind::SingleChoice { options }
            | QuestionKind::MultipleChoice { options, .. } = &question.kind
            {
                option_ids.extend(options.iter().map(|option| option.id));
            }
        }
        for binding in self.bindings.values() {
            let valid = match binding.category {
                Category::Question => question_ids.contains(&binding.id),
                Category::Option => option_ids.contains(&binding.id),
                _ => definitions.contains(&binding.id),
            };
            if !valid {
                return Err(failure(
                    self.source,
                    first_offset,
                    LogicErrorCode::InvalidReference,
                ));
            }
        }
        logic.compile(questions).map_err(|error| {
            let offset = self
                .locations
                .iter()
                .filter(|(path, _)| error.path.starts_with(path))
                .max_by_key(|(path, _)| path.len())
                .map_or(0, |(_, offset)| *offset);
            located(self.source, offset, error)
        })?;
        Ok(logic)
    }
}

/// 通过 pest 文法生成统一规则包，再执行身份绑定、类型与流程编译。
///
/// # Errors
/// 语法、引用、资源或语义无效时返回带行列及 AST 路径的诊断。
pub fn parse_logic_text(
    source: &str,
    questions: &[Question],
) -> Result<DocumentLogic, LogicTextError> {
    check_text_budget(source)?;
    let program = next(&mut parse(source, Rule::program)?);
    Compiler {
        source,
        bindings: BTreeMap::new(),
        locations: Vec::new(),
    }
    .compile(program, questions)
}

/// 二元运算符由 (op, 名称) 反查符号；与 TS 端 `binary` 表一一对应。
fn binary_symbol(op: &str, name: &str) -> Option<&'static str> {
    match (op, name) {
        ("any", _) => Some("||"),
        ("all", _) => Some("&&"),
        ("compare", "eq") => Some("=="),
        ("compare", "ne") => Some("!="),
        ("compare", "lt") => Some("<"),
        ("compare", "le") => Some("<="),
        ("compare", "gt") => Some(">"),
        ("compare", "ge") => Some(">="),
        ("arithmetic", "add") => Some("+"),
        ("arithmetic", "subtract") => Some("-"),
        ("arithmetic", "multiply") => Some("*"),
        ("arithmetic", "divide") => Some("/"),
        _ => None,
    }
}

/// 文本序列化器：别名与行缓冲按 TS `formatLogicText` 的顺序填充。
struct Formatter {
    aliases: BTreeMap<Uuid, String>,
    lines: Vec<String>,
}

impl Formatter {
    fn new(dsl_version: &Number, semantics_version: &Number) -> Self {
        Self {
            aliases: BTreeMap::new(),
            lines: vec![format!("dsl {dsl_version} semantics {semantics_version};")],
        }
    }

    fn bind(&mut self, category: &str, id: Uuid, alias: String) {
        self.lines
            .push(format!("bind {category} {alias} = \"{id}\";"));
        self.aliases.insert(id, alias);
    }

    fn bound(&self, id: Uuid) -> Result<&str, LogicError> {
        self.aliases
            .get(&id)
            .map(String::as_str)
            .ok_or_else(|| error(LogicErrorCode::InvalidReference, "$", None))
    }

    /// 未知绑定返回 `invalid_reference`；与 TS 端 `ref` 闭包语义一致。
    fn reference(&self, id: &Value) -> Result<&str, LogicError> {
        id.as_str()
            .and_then(crate::identity::parse)
            .ok_or_else(|| error(LogicErrorCode::InvalidReference, "$", None))
            .and_then(|id| self.bound(id))
    }

    /// 递归渲染表达式节点；未知结构返回 `invalid_syntax`，运算符按 TS `binary` 表反查。
    fn expression(&self, raw: &Value) -> Result<String, LogicError> {
        let invalid_syntax = || error(LogicErrorCode::InvalidSyntax, "$", None);
        let Some(fields) = raw.as_object() else {
            return Err(invalid_syntax());
        };
        let field = |name: &str| fields.get(name).ok_or_else(invalid_syntax);
        let text = |name: &str| field(name)?.as_str().ok_or_else(invalid_syntax);
        let quoted = |name: &str| serde_json::to_string(text(name)?).map_err(|_| invalid_syntax());
        let op = text("op")?;
        Ok(match op {
            "boolean" => match field("value")? {
                Value::Bool(true) => "true".into(),
                Value::Bool(false) => "false".into(),
                _ => return Err(invalid_syntax()),
            },
            "number" => format!("number({})", quoted("value")?),
            "text" => quoted("value")?,
            "answer" | "answered" => {
                format!("{}({})", op, self.reference(field("question_id")?)?)
            }
            "variable" => format!("variable({})", self.reference(field("variable_id")?)?),
            "selected" => format!(
                "selected({}, {})",
                self.reference(field("question_id")?)?,
                self.reference(field("option_id")?)?
            ),
            "contains_any" | "contains_all" => {
                let option_ids = field("option_ids")?.as_array().ok_or_else(invalid_syntax)?;
                let references = option_ids
                    .iter()
                    .map(|option_id| self.reference(option_id).map(str::to_owned))
                    .collect::<Result<Vec<_>, _>>()?;
                format!(
                    "{}({}, [{}])",
                    op,
                    self.reference(field("question_id")?)?,
                    references.join(", ")
                )
            }
            "not" => format!("!({})", self.expression(field("expression")?)?),
            "all" | "any" => {
                let expressions = field("expressions")?
                    .as_array()
                    .ok_or_else(invalid_syntax)?;
                let parts = expressions
                    .iter()
                    .map(|child| self.expression(child))
                    .collect::<Result<Vec<_>, _>>()?;
                format!("{}({})", op, parts.join(", "))
            }
            "coalesce" => format!(
                "coalesce({}, {})",
                self.expression(field("value")?)?,
                self.expression(field("fallback")?)?
            ),
            "length" => format!("length({})", self.expression(field("value")?)?),
            // 二元运算符按 (op, comparison/operator) 反查符号，无法反查即拒绝。
            _ => {
                let name = if op == "compare" {
                    text("comparison")?
                } else {
                    text("operator")?
                };
                let operator = binary_symbol(op, name).ok_or_else(invalid_syntax)?;
                format!(
                    "({} {} {})",
                    self.expression(field("left")?)?,
                    operator,
                    self.expression(field("right")?)?
                )
            }
        })
    }
}

/// 生成确定性的可读别名，绑定仍由 UUID 定义；常量原文与表达式分组保持不变。
///
/// # Errors
/// 编译预检失败、引用未绑定或表达式结构无法反查运算符时返回带 `$` 路径的领域错误。
pub fn format_logic_text(
    logic: &DocumentLogic,
    questions: &[Question],
) -> Result<String, LogicError> {
    logic.compile(questions)?;
    let mut formatter = Formatter::new(&logic.dsl_version, &logic.semantics_version);
    for (index, question) in questions.iter().enumerate() {
        formatter.bind("question", question.id, format!("q{}", index + 1));
        if let QuestionKind::SingleChoice { options }
        | QuestionKind::MultipleChoice { options, .. } = &question.kind
        {
            for (order, option) in options.iter().enumerate() {
                formatter.bind(
                    "option",
                    option.id,
                    format!("q{}_o{}", index + 1, order + 1),
                );
            }
        }
    }
    for (index, variable) in logic.variables.iter().enumerate() {
        formatter.bind("variable", variable.id, format!("v{}", index + 1));
    }
    if let Some(flow) = &logic.flow {
        for (index, node) in flow.nodes.iter().enumerate() {
            formatter.bind("node", node.id(), format!("n{}", index + 1));
        }
    }
    formatter.lines.push(String::new());
    for variable in &logic.variables {
        formatter.lines.push(format!(
            "let {}: {} = {};",
            formatter.bound(variable.id)?,
            variable.value_type.as_str(),
            formatter.expression(&variable.expression)?
        ));
    }
    for rule in &logic.rules {
        for (raw, keyword) in [
            (&rule.visible_if, "visible"),
            (&rule.required_if, "required"),
            (&rule.validate, "validate"),
        ] {
            if let Some(raw) = raw {
                formatter.lines.push(format!(
                    "{keyword} {} = {};",
                    formatter.bound(rule.question_id)?,
                    formatter.expression(raw)?
                ));
            }
        }
    }
    if let Some(flow) = &logic.flow {
        formatter
            .lines
            .push(format!("entry {};", formatter.bound(flow.entry)?));
        for node in &flow.nodes {
            match node {
                FlowNode::Page {
                    id,
                    question_ids,
                    next,
                } => {
                    let references = question_ids
                        .iter()
                        .map(|question_id| formatter.bound(*question_id).map(str::to_owned))
                        .collect::<Result<Vec<_>, _>>()?;
                    formatter.lines.push(format!(
                        "page {} [{}] -> {};",
                        formatter.bound(*id)?,
                        references.join(", "),
                        formatter.bound(*next)?
                    ));
                }
                FlowNode::End { id, outcome } => formatter.lines.push(format!(
                    "end {} = {};",
                    formatter.bound(*id)?,
                    match outcome {
                        FlowOutcome::Complete => "complete",
                        FlowOutcome::ScreenOut => "screen_out",
                    }
                )),
                FlowNode::Branch {
                    id,
                    branches,
                    otherwise,
                } => {
                    formatter
                        .lines
                        .push(format!("branch {} {{", formatter.bound(*id)?));
                    for branch in branches {
                        formatter.lines.push(format!(
                            "  when {} -> {};",
                            formatter.expression(&branch.when)?,
                            formatter.bound(branch.target)?
                        ));
                    }
                    formatter
                        .lines
                        .push(format!("  else -> {};", formatter.bound(*otherwise)?));
                    formatter.lines.push("}".into());
                }
            }
        }
    }
    formatter.lines.push(String::new());
    Ok(formatter.lines.join("\n"))
}
