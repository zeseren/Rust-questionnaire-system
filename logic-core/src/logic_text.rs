//! 文本文法交给 pest；先限制解析资源，再绑定身份并复用统一 AST 与语义编译。

use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    fmt::Write as _,
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
use crate::logic::{LogicError, LogicErrorCode, LogicValueType, check_budget, error};
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

/// 规则包中需要回映射源位置的路径片段；仅在编译失败时格式化为字符串。
#[derive(Clone, Copy)]
enum Location {
    Variable(usize),
    Rule { index: usize, field: RuleField },
    Flow,
    Node(usize),
}

impl Location {
    /// 生成与 AST 错误路径匹配的前缀；成功路径不分配这些字符串。
    fn path(self) -> String {
        match self {
            Self::Variable(index) => format!("$.logic.variables[{index}]"),
            Self::Rule { index, field } => format!("$.logic.rules[{index}].{}", field.as_str()),
            Self::Flow => "$.logic.flow".into(),
            Self::Node(index) => format!("$.logic.flow.nodes[{index}]"),
        }
    }
}

#[derive(Clone, Copy)]
enum RuleField {
    Visible,
    Required,
    Validate,
}

impl RuleField {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Visible => "visible_if",
            Self::Required => "required_if",
            Self::Validate => "validate",
        }
    }
}

/// 已构建子树的预算摘要：对象层数与对象节点数。
#[derive(Clone, Copy)]
struct Budget {
    depth: usize,
    nodes: usize,
}

impl Budget {
    /// 单个不含子表达式的对象。
    const LEAF: Self = Self { depth: 1, nodes: 1 };

    /// 由子节点汇总父对象：深度加一，节点数为父节点与子节点之和。
    fn parent(children: impl IntoIterator<Item = Self>) -> Self {
        let mut depth = 0;
        let mut nodes = 1;
        for child in children {
            depth = depth.max(child.depth);
            nodes += child.nodes;
        }
        Self {
            depth: depth + 1,
            nodes,
        }
    }

    /// 越界时与 `check_raw_budget` 返回同一分类，并在构造位置报告。
    fn checked(self, source: &str, offset: usize) -> Result<Self, LogicTextError> {
        check_budget(self.depth, self.nodes).map_err(|error| located(source, offset, error))?;
        Ok(self)
    }
}

struct Compiler<'a> {
    source: &'a str,
    bindings: BTreeMap<&'a str, Binding>,
    rule_indices: BTreeMap<Uuid, usize>,
    locations: Vec<(Location, usize)>,
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

    fn expression(&self, pair: Pair<'a, Rule>) -> Result<Value, LogicTextError> {
        self.build(pair).map(|(value, _)| value)
    }

    /// Pratt 解析同时维护每个子树的 `(depth, nodes)`，
    /// 避免每次构造二元节点都重新遍历整棵左子树。
    fn build(&self, pair: Pair<'a, Rule>) -> Result<(Value, Budget), LogicTextError> {
        if pair.as_rule() != Rule::expression {
            return Err(self.failure(&pair, LogicErrorCode::InvalidSyntax));
        }
        PRECEDENCE
            .map_primary(|pair| self.primary(pair))
            .map_prefix(|op, value| {
                let (value, budget) = value?;
                let budget = Budget::parent([budget]).checked(self.source, op.as_span().start())?;
                Ok((json!({"op":"not", "expression":value}), budget))
            })
            .map_infix(|left, op, right| {
                let (left, left_budget) = left?;
                let (right, right_budget) = right?;
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
                let budget = Budget::parent([left_budget, right_budget])
                    .checked(self.source, op.as_span().start())?;
                Ok((value, budget))
            })
            .parse(pair.into_inner())
    }

    fn primary(&self, pair: Pair<'a, Rule>) -> Result<(Value, Budget), LogicTextError> {
        let value = match pair.as_rule() {
            Rule::number | Rule::negative_number => {
                json!({"op":"number", "value":pair.as_str()})
            }
            Rule::string => json!({"op":"text", "value":string(self.source, &pair)?}),
            Rule::boolean => json!({"op":"boolean", "value":pair.as_str() == "true"}),
            Rule::identifier => {
                json!({"op":"variable", "variable_id":self.reference(pair, Category::Variable)?})
            }
            Rule::group => return self.build(next(&mut pair.into_inner())),
            Rule::call => return self.call(pair),
            _ => return Err(self.failure(&pair, LogicErrorCode::InvalidSyntax)),
        };
        Ok((value, Budget::LEAF))
    }

    fn call(&self, pair: Pair<'a, Rule>) -> Result<(Value, Budget), LogicTextError> {
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
        let (value, budget) = match op {
            "answer" | "answered" => (
                json!({"op":op,"question_id":self.reference(argument(0), Category::Question)?}),
                Budget::LEAF,
            ),
            "variable" => (
                json!({"op":op,"variable_id":self.reference(argument(0), Category::Variable)?}),
                Budget::LEAF,
            ),
            "number" => {
                let text = self.atom(argument(0))?;
                if text.as_rule() != Rule::string {
                    return Err(self.failure(&text, LogicErrorCode::InvalidSyntax));
                }
                (
                    json!({"op":op,"value":string(self.source, &text)?}),
                    Budget::LEAF,
                )
            }
            "selected" => (
                json!({
                    "op":op,
                    "question_id":self.reference(argument(0), Category::Question)?,
                    "option_id":self.reference(argument(1), Category::Option)?
                }),
                Budget::LEAF,
            ),
            "contains_any" | "contains_all" => (
                json!({
                    "op":op,
                    "question_id":self.reference(argument(0), Category::Question)?,
                    "option_ids":self.references(argument(1), Category::Option)?
                }),
                Budget::LEAF,
            ),
            "all" | "any" => {
                let mut expressions = Vec::with_capacity(arguments.len());
                let mut depth = 0;
                let mut nodes = 1;
                // 每加入一个参数就检查合计预算，与逐参数累计的原有极限行为一致。
                for argument in arguments {
                    let (value, child) = self.build(argument)?;
                    depth = depth.max(child.depth);
                    nodes += child.nodes;
                    let total = Budget {
                        depth: depth + 1,
                        nodes,
                    };
                    total.checked(self.source, offset)?;
                    expressions.push(value);
                }
                (
                    json!({"op":op,"expressions":expressions}),
                    Budget {
                        depth: depth + 1,
                        nodes,
                    },
                )
            }
            "coalesce" => {
                let (value, value_budget) = self.build(argument(0))?;
                let (fallback, fallback_budget) = self.build(argument(1))?;
                (
                    json!({"op":op,"value":value,"fallback":fallback}),
                    Budget::parent([value_budget, fallback_budget]),
                )
            }
            _ => {
                let (value, child) = self.build(argument(0))?;
                (json!({"op":op,"value":value}), Budget::parent([child]))
            }
        };
        let budget = budget.checked(self.source, offset)?;
        Ok((value, budget))
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
                    let source = self.source;
                    let id = crate::identity::parse(&raw).ok_or_else(|| {
                        failure(source, location, LogicErrorCode::InvalidReference)
                    })?;
                    match self.bindings.entry(alias.as_str()) {
                        Entry::Occupied(_) => {
                            return Err(failure(
                                source,
                                location,
                                LogicErrorCode::InvalidReference,
                            ));
                        }
                        Entry::Vacant(entry) => {
                            if !bound_ids.insert(id) {
                                return Err(failure(
                                    source,
                                    location,
                                    LogicErrorCode::InvalidReference,
                                ));
                            }
                            entry.insert(Binding { category, id });
                        }
                    }
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
                    self.locations
                        .push((Location::Variable(logic.variables.len()), offset));
                    logic.variables.push(ComputedVariable {
                        id,
                        value_type,
                        expression: self.expression(next(&mut fields))?,
                    });
                }
                Rule::question_rule => {
                    let rule_kind = next(&mut fields);
                    let question_id = self.reference(next(&mut fields), Category::Question)?;
                    let index = match self.rule_indices.get(&question_id) {
                        Some(index) => *index,
                        None => {
                            logic.rules.push(QuestionRule {
                                question_id,
                                visible_if: None,
                                required_if: None,
                                validate: None,
                            });
                            let index = logic.rules.len() - 1;
                            self.rule_indices.insert(question_id, index);
                            index
                        }
                    };
                    let (slot, field) = match rule_kind.as_str() {
                        "visible" => (&mut logic.rules[index].visible_if, RuleField::Visible),
                        "required" => (&mut logic.rules[index].required_if, RuleField::Required),
                        _ => (&mut logic.rules[index].validate, RuleField::Validate),
                    };
                    if slot.is_some() {
                        return Err(failure(self.source, offset, LogicErrorCode::InvalidSyntax));
                    }
                    self.locations
                        .push((Location::Rule { index, field }, offset));
                    *slot = Some(self.expression(next(&mut fields))?);
                }
                Rule::entry => {
                    if entry.is_some() {
                        return Err(failure(self.source, offset, LogicErrorCode::InvalidSyntax));
                    }
                    next(&mut fields);
                    entry = Some(self.reference(next(&mut fields), Category::Node)?);
                    self.locations.push((Location::Flow, offset));
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
                    self.locations.push((Location::Node(nodes.len()), offset));
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
            // 成功路径只保留轻量位置枚举；仅在报错时格式化路径并选最长前缀。
            let offset = self
                .locations
                .iter()
                .filter_map(|(location, offset)| {
                    let path = location.path();
                    error
                        .path
                        .starts_with(path.as_str())
                        .then_some((path, *offset))
                })
                .max_by_key(|(path, _)| path.len())
                .map_or(0, |(_, offset)| offset);
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
        rule_indices: BTreeMap::new(),
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

/// 文本序列化器：别名与单缓冲区按 TS `formatLogicText` 的顺序填充。
struct Formatter {
    aliases: BTreeMap<Uuid, String>,
    output: String,
}

impl Formatter {
    fn new(dsl_version: &Number, semantics_version: &Number) -> Self {
        let mut output = String::with_capacity(256);
        write!(output, "dsl {dsl_version} semantics {semantics_version};")
            .expect("写入 String 不会失败");
        Self {
            aliases: BTreeMap::new(),
            output,
        }
    }

    fn bind(&mut self, category: &str, id: Uuid, alias: String) {
        self.output.push('\n');
        write!(self.output, "bind {category} {alias} = \"{id}\";").expect("写入 String 不会失败");
        self.aliases.insert(id, alias);
    }

    fn write_variable(&mut self, variable: &ComputedVariable) -> Result<(), LogicError> {
        let alias = alias(&self.aliases, variable.id)?;
        self.output.push('\n');
        write!(
            self.output,
            "let {alias}: {} = ",
            variable.value_type.as_str()
        )
        .expect("写入 String 不会失败");
        write_expression(&self.aliases, &mut self.output, &variable.expression)?;
        self.output.push(';');
        Ok(())
    }

    fn write_rule(
        &mut self,
        keyword: &str,
        question_id: Uuid,
        raw: &Value,
    ) -> Result<(), LogicError> {
        let alias = alias(&self.aliases, question_id)?;
        self.output.push('\n');
        write!(self.output, "{keyword} {alias} = ").expect("写入 String 不会失败");
        write_expression(&self.aliases, &mut self.output, raw)?;
        self.output.push(';');
        Ok(())
    }

    fn write_flow(&mut self, flow: &FlowDefinition) -> Result<(), LogicError> {
        let entry = alias(&self.aliases, flow.entry)?;
        self.output.push('\n');
        write!(self.output, "entry {entry};").expect("写入 String 不会失败");
        for node in &flow.nodes {
            self.output.push('\n');
            match node {
                FlowNode::Page {
                    id,
                    question_ids,
                    next,
                } => {
                    let node = alias(&self.aliases, *id)?;
                    let next = alias(&self.aliases, *next)?;
                    write!(self.output, "page {node} [").expect("写入 String 不会失败");
                    for (index, question_id) in question_ids.iter().enumerate() {
                        if index > 0 {
                            self.output.push_str(", ");
                        }
                        self.output.push_str(alias(&self.aliases, *question_id)?);
                    }
                    write!(self.output, "] -> {next};").expect("写入 String 不会失败");
                }
                FlowNode::End { id, outcome } => {
                    let node = alias(&self.aliases, *id)?;
                    let outcome = match outcome {
                        FlowOutcome::Complete => "complete",
                        FlowOutcome::ScreenOut => "screen_out",
                    };
                    write!(self.output, "end {node} = {outcome};").expect("写入 String 不会失败");
                }
                FlowNode::Branch {
                    id,
                    branches,
                    otherwise,
                } => {
                    let node = alias(&self.aliases, *id)?;
                    write!(self.output, "branch {node} {{").expect("写入 String 不会失败");
                    for branch in branches {
                        self.output.push_str("\n  when ");
                        write_expression(&self.aliases, &mut self.output, &branch.when)?;
                        self.output.push_str(" -> ");
                        self.output.push_str(alias(&self.aliases, branch.target)?);
                        self.output.push(';');
                    }
                    self.output.push_str("\n  else -> ");
                    self.output.push_str(alias(&self.aliases, *otherwise)?);
                    self.output.push_str(";\n}");
                }
            }
        }
        Ok(())
    }
}

/// 查找已绑定别名；未知绑定返回 `invalid_reference`。
fn alias(aliases: &BTreeMap<Uuid, String>, id: Uuid) -> Result<&str, LogicError> {
    aliases
        .get(&id)
        .map(String::as_str)
        .ok_or_else(|| error(LogicErrorCode::InvalidReference, "$", None))
}

/// 解析引用值并转换为别名；与 TS 端 `ref` 闭包语义一致。
fn reference<'a>(aliases: &'a BTreeMap<Uuid, String>, id: &Value) -> Result<&'a str, LogicError> {
    id.as_str()
        .and_then(crate::identity::parse)
        .ok_or_else(|| error(LogicErrorCode::InvalidReference, "$", None))
        .and_then(|id| alias(aliases, id))
}

/// 递归写入表达式节点；未知结构返回 `invalid_syntax`，运算符按 TS `binary` 表反查。
fn write_expression(
    aliases: &BTreeMap<Uuid, String>,
    output: &mut String,
    raw: &Value,
) -> Result<(), LogicError> {
    let invalid_syntax = || error(LogicErrorCode::InvalidSyntax, "$", None);
    let Some(fields) = raw.as_object() else {
        return Err(invalid_syntax());
    };
    let field = |name: &str| fields.get(name).ok_or_else(invalid_syntax);
    let text = |name: &str| field(name)?.as_str().ok_or_else(invalid_syntax);
    let op = text("op")?;
    match op {
        "boolean" => match field("value")? {
            Value::Bool(true) => output.push_str("true"),
            Value::Bool(false) => output.push_str("false"),
            _ => return Err(invalid_syntax()),
        },
        "number" => {
            let quoted = serde_json::to_string(text("value")?).map_err(|_| invalid_syntax())?;
            write!(output, "number({quoted})").expect("写入 String 不会失败");
        }
        "text" => {
            let quoted = serde_json::to_string(text("value")?).map_err(|_| invalid_syntax())?;
            output.push_str(&quoted);
        }
        "answer" | "answered" => {
            let question = reference(aliases, field("question_id")?)?;
            write!(output, "{op}({question})").expect("写入 String 不会失败");
        }
        "variable" => {
            let variable = reference(aliases, field("variable_id")?)?;
            write!(output, "variable({variable})").expect("写入 String 不会失败");
        }
        "selected" => {
            let question = reference(aliases, field("question_id")?)?;
            let option = reference(aliases, field("option_id")?)?;
            write!(output, "selected({question}, {option})").expect("写入 String 不会失败");
        }
        "contains_any" | "contains_all" => {
            let option_ids = field("option_ids")?.as_array().ok_or_else(invalid_syntax)?;
            let question = reference(aliases, field("question_id")?)?;
            write!(output, "{op}({question}, [").expect("写入 String 不会失败");
            for (index, option_id) in option_ids.iter().enumerate() {
                if index > 0 {
                    output.push_str(", ");
                }
                output.push_str(reference(aliases, option_id)?);
            }
            output.push_str("])");
        }
        "not" => {
            output.push_str("!(");
            write_expression(aliases, output, field("expression")?)?;
            output.push(')');
        }
        "all" | "any" => {
            let expressions = field("expressions")?
                .as_array()
                .ok_or_else(invalid_syntax)?;
            output.push_str(op);
            output.push('(');
            for (index, child) in expressions.iter().enumerate() {
                if index > 0 {
                    output.push_str(", ");
                }
                write_expression(aliases, output, child)?;
            }
            output.push(')');
        }
        "coalesce" => {
            output.push_str("coalesce(");
            write_expression(aliases, output, field("value")?)?;
            output.push_str(", ");
            write_expression(aliases, output, field("fallback")?)?;
            output.push(')');
        }
        "length" => {
            output.push_str("length(");
            write_expression(aliases, output, field("value")?)?;
            output.push(')');
        }
        _ => {
            // 二元运算符按 (op, comparison/operator) 反查符号，无法反查即拒绝。
            let name = if op == "compare" {
                text("comparison")?
            } else {
                text("operator")?
            };
            let operator = binary_symbol(op, name).ok_or_else(invalid_syntax)?;
            output.push('(');
            write_expression(aliases, output, field("left")?)?;
            output.push(' ');
            output.push_str(operator);
            output.push(' ');
            write_expression(aliases, output, field("right")?)?;
            output.push(')');
        }
    }
    Ok(())
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
    // 绑定与定义之间、以及输出末尾各保留一个空行，对应原实现的 `lines.join("\n")`。
    formatter.output.push('\n');
    for variable in &logic.variables {
        formatter.write_variable(variable)?;
    }
    for rule in &logic.rules {
        for (raw, keyword) in [
            (&rule.visible_if, "visible"),
            (&rule.required_if, "required"),
            (&rule.validate, "validate"),
        ] {
            if let Some(raw) = raw {
                formatter.write_rule(keyword, rule.question_id, raw)?;
            }
        }
    }
    if let Some(flow) = &logic.flow {
        formatter.write_flow(flow)?;
    }
    formatter.output.push('\n');
    Ok(formatter.output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn 位置(source: &str, offset: usize) -> (usize, usize) {
        let 错误 = located(
            source,
            offset,
            error(LogicErrorCode::InvalidSyntax, "$", None),
        );
        (错误.line, 错误.column)
    }

    #[test]
    fn 定位按字节偏移换算行列() {
        // LF 与 CRLF 的行号都只统计 `\n`，列号从 1 开始。
        assert_eq!(位置("abc\ndef", 0), (1, 1));
        assert_eq!(位置("abc\ndef", 3), (1, 4));
        assert_eq!(位置("abc\ndef", 4), (2, 1));
        assert_eq!(位置("abc\ndef", 6), (2, 3));
        assert_eq!(位置("ab\r\ncd", 4), (2, 1));
        assert_eq!(位置("ab\r\ncd", 6), (2, 3));

        // 列号按 Unicode 标量计数，中文占三字节、Emoji 占四字节但都只算一列。
        assert_eq!(位置("中文😀x", 10), (1, 4));
        assert_eq!(位置("a\n中文😀", 12), (2, 4));
    }

    #[test]
    fn 文本预算超限错误定位到精确行列() {
        // 字节预算：多一个字节立即拒绝并定位到首行首列。
        let 错误 = check_text_budget(&" ".repeat(1_048_577)).unwrap_err();
        assert_eq!(错误.error.code, LogicErrorCode::LimitExceeded);
        assert_eq!((错误.line, 错误.column), (1, 1));

        // 64 层括号使深度超限，必须定位到第 64 个左括号。
        let 超限 = format!("{}true{}", "(".repeat(64), ")".repeat(64));
        let 错误 = check_text_budget(&超限).unwrap_err();
        assert_eq!(错误.error.code, LogicErrorCode::LimitExceeded);
        assert_eq!((错误.line, 错误.column), (1, 64));

        // 前缀 `!` 与括号共用同一深度预算，同样定位到第 64 个字符。
        let 错误 = check_text_budget(&format!("{}true", "!".repeat(64))).unwrap_err();
        assert_eq!(错误.error.code, LogicErrorCode::LimitExceeded);
        assert_eq!((错误.line, 错误.column), (1, 64));
    }
}
