//! 表达式必须先通过类型与引用检查；求值不访问外部状态，也不把错误当作缺失值。

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::domain::{
    MAX_CHOICES, MAX_QUESTIONS, MAX_TEXT_LENGTH, Question, QuestionKind, canonical_zero,
    finite_number,
};

const MAX_NODES: usize = 256;
const MAX_DEPTH: usize = 16;

/// 两端共享的稳定错误分类；文案不参与分支判断。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogicErrorCode {
    /// JSON 结构或字段不符合契约。
    InvalidSyntax,
    /// 不支持该 DSL 或执行语义版本。
    UnsupportedVersion,
    /// 表达式超出资源预算。
    LimitExceeded,
    /// 题目或选项引用不存在或不唯一。
    InvalidReference,
    /// 运算符与操作数类型不匹配。
    TypeMismatch,
    /// 数字常量无效或失真。
    InvalidNumber,
    /// 上下文不符合已经绑定的题型。
    InvalidAnswer,
    /// 算术结果不是有限数字或发生除零。
    ArithmeticError,
    /// 计算变量形成循环依赖。
    DependencyCycle,
    /// 题目规则依赖自身或后续题目。
    ForwardReference,
    /// 流程结构无效或存在回跳。
    InvalidFlow,
    /// 分支汇合后的依赖未明确处理缺失。
    UnavailableDependency,
    /// 当前状态不接受此事件。
    InvalidEvent,
    /// 必填答案缺失。
    RequiredAnswer,
    /// 自定义答案约束不满足。
    ConstraintFailed,
    /// 条件尚不能确定。
    UnknownCondition,
    /// 提交携带不适用题目的答案。
    InapplicableAnswer,
    /// 筛选结束尚不接受持久化提交。
    ScreenedOut,
}

/// 规则错误保留 AST 路径与相关题目，但不回显答案。
#[derive(Debug, thiserror::Error, Serialize)]
#[error("{message}（{path}）")]
pub struct LogicError {
    /// 稳定错误代码。
    pub code: LogicErrorCode,
    /// 从表达式包根节点开始的字段路径。
    pub path: String,
    /// 涉及的题目；纯表达式错误没有题目。
    pub question_id: Option<Uuid>,
    /// 中文诊断信息。
    pub message: &'static str,
}

impl LogicError {
    pub(crate) fn relocate(mut self, path: &str) -> Self {
        self.path = self.path.replacen("$.expression", path, 1);
        self
    }
}

pub(crate) fn error(code: LogicErrorCode, path: &str, question_id: Option<Uuid>) -> LogicError {
    let message = match code {
        LogicErrorCode::InvalidSyntax => "规则结构或字段无效",
        LogicErrorCode::UnsupportedVersion => "不支持此规则版本",
        LogicErrorCode::LimitExceeded => "规则超出容量限制",
        LogicErrorCode::InvalidReference => "规则引用的题目或选项无效",
        LogicErrorCode::TypeMismatch => "规则的操作数类型不匹配",
        LogicErrorCode::InvalidNumber => "规则数字无效或无法精确表示",
        LogicErrorCode::InvalidAnswer => "规则上下文包含无效答案",
        LogicErrorCode::ArithmeticError => "规则计算发生除零或数值溢出",
        LogicErrorCode::DependencyCycle => "计算变量存在循环依赖",
        LogicErrorCode::ForwardReference => "题目规则不能依赖自身或后续题目",
        LogicErrorCode::InvalidFlow => "流程结构无效、不可达或包含回跳",
        LogicErrorCode::UnavailableDependency => "此依赖可能被分支绕过，请明确处理缺失值",
        LogicErrorCode::InvalidEvent => "当前填写状态不接受此操作",
        LogicErrorCode::RequiredAnswer => "此题为必填项",
        LogicErrorCode::ConstraintFailed => "答案不符合自定义约束",
        LogicErrorCode::UnknownCondition => "条件尚不能确定，请补全相关答案",
        LogicErrorCode::InapplicableAnswer => "提交包含隐藏或未进入分支的题目，请核对当前路径",
        LogicErrorCode::ScreenedOut => "筛选结束不产生正式答卷",
    };
    LogicError {
        code,
        path: path.into(),
        question_id,
        message,
    }
}

/// 编译阶段确定的类型；缺失值保留原表达式的静态类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LogicValueType {
    /// 布尔条件。
    Boolean,
    /// 有限 binary64 数字。
    Number,
    /// Unicode 文本。
    Text,
    /// 单选身份。
    Choice,
    /// 多选集合。
    Choices,
}

impl LogicValueType {
    /// 稳定的 snake_case 名称；序列化名与文本 DSL 共用这一处事实源。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Boolean => "boolean",
            Self::Number => "number",
            Self::Text => "text",
            Self::Choice => "choice",
            Self::Choices => "choices",
        }
    }

    /// 从文本 DSL 类型名解析值类型。
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "boolean" => Some(Self::Boolean),
            "number" => Some(Self::Number),
            "text" => Some(Self::Text),
            "choice" => Some(Self::Choice),
            "choices" => Some(Self::Choices),
            _ => None,
        }
    }
}

/// 求值结果；布尔位置的 Missing 对应三值逻辑中的 unknown。
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum LogicValue {
    /// 无适用答案。
    Missing,
    /// 布尔结果。
    Boolean(bool),
    /// 有限数字；零统一为正零。
    Number(f64),
    /// 不做隐式裁剪或归一化的文本。
    Text(String),
    /// 单选选项 ID。
    Choice(Uuid),
    /// 排序后的无重复选项集合。
    Choices(Vec<Uuid>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Program {
    dsl_version: Number,
    semantics_version: Number,
    expression: Expression,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Expression {
    Boolean {
        value: bool,
    },
    Number {
        value: String,
    },
    Text {
        value: String,
    },
    Answer {
        question_id: String,
    },
    Answered {
        question_id: String,
    },
    Variable {
        variable_id: String,
    },
    Selected {
        question_id: String,
        option_id: String,
    },
    ContainsAny {
        question_id: String,
        option_ids: Vec<String>,
    },
    ContainsAll {
        question_id: String,
        option_ids: Vec<String>,
    },
    Not {
        expression: Box<Expression>,
    },
    All {
        expressions: Vec<Expression>,
    },
    Any {
        expressions: Vec<Expression>,
    },
    Compare {
        comparison: Comparison,
        left: Box<Expression>,
        right: Box<Expression>,
    },
    Coalesce {
        value: Box<Expression>,
        fallback: Box<Expression>,
    },
    Length {
        value: Box<Expression>,
    },
    Arithmetic {
        operator: Arithmetic,
        left: Box<Expression>,
        right: Box<Expression>,
    },
}

#[derive(Clone, Copy, Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
enum Comparison {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Copy, Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
enum Arithmetic {
    Add,
    Subtract,
    Multiply,
    Divide,
}

#[derive(Debug)]
struct Binding {
    value_type: LogicValueType,
    /// 选项身份；每题至多 [`MAX_CHOICES`] 个，线性查找即可。
    options: Vec<Uuid>,
    variable: bool,
}

#[derive(Debug)]
struct Node {
    kind: NodeKind,
    value_type: LogicValueType,
    path: String,
}

#[derive(Debug)]
enum NodeKind {
    Literal(LogicValue),
    Answer(Uuid),
    Answered(Uuid),
    Selected(Uuid, Vec<Uuid>, bool),
    Not(Box<Node>),
    Logical(Vec<Node>, bool),
    Compare(Comparison, Box<Node>, Box<Node>),
    Coalesce(Box<Node>, Box<Node>),
    Length(Box<Node>),
    Arithmetic(Arithmetic, Box<Node>, Box<Node>),
}

/// 只有编译入口能构造的规则；绑定与常量不会在执行期间被外部修改。
#[derive(Debug)]
pub struct CompiledExpression {
    root: Node,
    bindings: Arc<BTreeMap<Uuid, Binding>>,
    dependencies: BTreeSet<Uuid>,
    variable_dependencies: BTreeSet<Uuid>,
}

impl CompiledExpression {
    /// 返回结果的静态类型，供显示规则或计算字段检查。
    #[must_use]
    pub fn value_type(&self) -> LogicValueType {
        self.root.value_type
    }

    /// 返回按 UUID 排序的直接依赖，供后续文档编译器构建依赖图。
    pub fn dependencies(&self) -> impl Iterator<Item = Uuid> + '_ {
        self.dependencies.iter().copied()
    }

    pub(crate) fn variable_dependencies(&self) -> impl Iterator<Item = Uuid> + '_ {
        self.variable_dependencies.iter().copied()
    }

    /// 根节点是布尔字面量时返回其值；流程分支用它识别常量条件。
    pub(crate) fn literal_boolean(&self) -> Option<bool> {
        match &self.root.kind {
            NodeKind::Literal(LogicValue::Boolean(value)) => Some(*value),
            _ => None,
        }
    }

    /// 求值时必然读取的题目集合；计算变量引用展开为其依赖的题目。
    ///
    /// `variables` 由文档编译器按拓扑序完整提供：任何被引用的计算变量都必须已展开。
    /// 缺失即内部不变量违约，应当显式失败而不是静默当作“无依赖”掩盖依赖。
    pub(crate) fn required_reads(
        &self,
        variables: &BTreeMap<Uuid, BTreeSet<Uuid>>,
    ) -> BTreeSet<Uuid> {
        self.root.required_reads(&self.bindings, variables)
    }

    /// 对已按问卷规则规范化、且已排除不适用答案的上下文求值。
    ///
    /// 本入口防御性检查引用答案的类型、数字和选项身份，不代替问卷的必填、
    /// 长度、范围或路径校验。未引用的答案不会参与求值。
    ///
    /// # 错误
    /// 引用答案类型错误、除零或非有限结果时返回结构化错误。
    pub fn evaluate(&self, answers: &BTreeMap<Uuid, Value>) -> Result<LogicValue, LogicError> {
        self.evaluate_with_variables(answers, &BTreeMap::new())
    }

    pub(crate) fn evaluate_with_variables(
        &self,
        answers: &BTreeMap<Uuid, Value>,
        variables: &BTreeMap<Uuid, LogicValue>,
    ) -> Result<LogicValue, LogicError> {
        // 只在此处检查变量是否齐备；求值只在短路路径上按需解析变量。
        if self
            .variable_dependencies
            .iter()
            .any(|id| !variables.contains_key(id))
        {
            return Err(error(LogicErrorCode::InvalidReference, "$.variables", None));
        }
        self.evaluate_resolving(answers, &mut |id| Ok(variables[&id].clone()))
    }

    pub(crate) fn evaluate_resolving(
        &self,
        answers: &BTreeMap<Uuid, Value>,
        variable: &mut dyn FnMut(Uuid) -> Result<LogicValue, LogicError>,
    ) -> Result<LogicValue, LogicError> {
        self.root.evaluate(&mut |id| {
            if self.variable_dependencies.contains(&id) {
                variable(id)
            } else {
                read_answer(answers.get(&id), &self.bindings[&id], id)
            }
        })
    }
}

/// 将 JSON 规则绑定到问卷题目，检查结构、版本、类型、引用和资源限制。
///
/// # 错误
/// 未知字段/版本、非法常量、无效引用、类型不匹配及超限均拒绝编译。
pub fn compile_expression(
    raw: &Value,
    questions: &[Question],
) -> Result<CompiledExpression, LogicError> {
    // 在递归反序列化前限制 AST 深度，避免无效规则构造深层递归类型。
    check_raw_budget(raw.get("expression"), 1, &mut 0)?;
    let program =
        Program::deserialize(raw).map_err(|_| error(LogicErrorCode::InvalidSyntax, "$", None))?;
    CompilationContext::new(
        &program.dsl_version,
        &program.semantics_version,
        questions,
        &BTreeMap::new(),
    )?
    .compile_parsed(program.expression)
}

/// 同一文档只构建一次绑定；表达式共享只读表，不复制题目与选项集合。
pub(crate) struct CompilationContext {
    bindings: Arc<BTreeMap<Uuid, Binding>>,
}

pub(crate) fn validate_versions(
    dsl_version: &Number,
    semantics_version: &Number,
) -> Result<(), LogicError> {
    for version in [dsl_version, semantics_version] {
        let Some(value) = version.as_u64() else {
            return Err(error(LogicErrorCode::InvalidSyntax, "$", None));
        };
        if value != 1 {
            return Err(error(LogicErrorCode::UnsupportedVersion, "$", None));
        }
    }
    Ok(())
}

impl CompilationContext {
    pub(crate) fn new(
        dsl_version: &Number,
        semantics_version: &Number,
        questions: &[Question],
        variables: &BTreeMap<Uuid, LogicValueType>,
    ) -> Result<Self, LogicError> {
        validate_versions(dsl_version, semantics_version)?;
        if questions.len() > MAX_QUESTIONS {
            return Err(error(LogicErrorCode::LimitExceeded, "$.questions", None));
        }
        let mut bindings = BTreeMap::new();
        for question in questions {
            let options = question.kind.options().unwrap_or(&[]);
            let value_type = match &question.kind {
                QuestionKind::SingleChoice { .. } => LogicValueType::Choice,
                QuestionKind::MultipleChoice { .. } => LogicValueType::Choices,
                QuestionKind::Text { .. } | QuestionKind::Textarea { .. } => LogicValueType::Text,
                QuestionKind::Number { .. } | QuestionKind::Rating { .. } => LogicValueType::Number,
            };
            // 排序只为让重复身份相邻，便于一次扫描发现。
            let mut ids: Vec<Uuid> = options.iter().map(|option| option.id).collect();
            ids.sort_unstable();
            if question.id.is_nil()
                || bindings.contains_key(&question.id)
                || ids.windows(2).any(|pair| pair[0] == pair[1])
                || ids.contains(&Uuid::nil())
            {
                return Err(error(
                    LogicErrorCode::InvalidReference,
                    "$.questions",
                    Some(question.id),
                ));
            }
            if options.len() > MAX_CHOICES {
                return Err(error(
                    LogicErrorCode::LimitExceeded,
                    "$.questions",
                    Some(question.id),
                ));
            }
            bindings.insert(
                question.id,
                Binding {
                    value_type,
                    options: ids,
                    variable: false,
                },
            );
        }
        for (id, value_type) in variables {
            if bindings.contains_key(id) {
                return Err(error(LogicErrorCode::InvalidReference, "$.variables", None));
            }
            bindings.insert(
                *id,
                Binding {
                    value_type: *value_type,
                    options: Vec::new(),
                    variable: true,
                },
            );
        }
        Ok(Self {
            bindings: Arc::new(bindings),
        })
    }

    pub(crate) fn compile(&self, raw: &Value) -> Result<CompiledExpression, LogicError> {
        // 预算由调用方在反序列化前统一执行：DocumentLogic::compile 覆盖变量、规则与分支，
        // compile_expression 覆盖单个表达式包。
        let expression = Expression::deserialize(raw)
            .map_err(|_| error(LogicErrorCode::InvalidSyntax, "$", None))?;
        self.compile_parsed(expression)
    }

    fn compile_parsed(&self, expression: Expression) -> Result<CompiledExpression, LogicError> {
        let mut dependencies = BTreeSet::new();
        let root = compile_node(
            expression,
            "$.expression".into(),
            &self.bindings,
            &mut dependencies,
        )?;
        let variable_dependencies = dependencies
            .iter()
            .copied()
            .filter(|id| self.bindings[id].variable)
            .collect();
        dependencies.retain(|id| !self.bindings[id].variable);
        Ok(CompiledExpression {
            root,
            bindings: Arc::clone(&self.bindings),
            dependencies,
            variable_dependencies,
        })
    }
}

/// 表达式的深度与节点预算共用同一事实源：文本解析器维护子树摘要时也调用这里。
pub(crate) fn check_budget(depth: usize, nodes: usize) -> Result<(), LogicError> {
    if depth > MAX_DEPTH || nodes > MAX_NODES {
        return Err(error(LogicErrorCode::LimitExceeded, "$.expression", None));
    }
    Ok(())
}

pub(crate) fn check_raw_budget(
    raw: Option<&Value>,
    depth: usize,
    count: &mut usize,
) -> Result<(), LogicError> {
    *count += 1;
    check_budget(depth, *count)?;
    if let Some(Value::Object(fields)) = raw {
        for key in ["expression", "left", "right", "fallback", "value"] {
            if let Some(child @ Value::Object(_)) = fields.get(key) {
                check_raw_budget(Some(child), depth + 1, count)?;
            }
        }
        if let Some(Value::Array(children)) = fields.get("expressions") {
            for child in children {
                check_raw_budget(Some(child), depth + 1, count)?;
            }
        }
    }
    Ok(())
}

pub(crate) fn reference(raw: &str, path: &str) -> Result<Uuid, LogicError> {
    crate::identity::parse(raw).ok_or_else(|| error(LogicErrorCode::InvalidReference, path, None))
}

fn numeric(raw: &str) -> Option<f64> {
    // 数字常量只能是无空白的 JSON 数字；借用既有精度规则，不改变答案契约。
    if raw.trim() != raw {
        return None;
    }
    let value: Value = serde_json::from_str(raw).ok()?;
    finite_number(&value, Uuid::nil()).ok().map(canonical_zero)
}

fn compile_node(
    expr: Expression,
    path: String,
    bindings: &BTreeMap<Uuid, Binding>,
    dependencies: &mut BTreeSet<Uuid>,
) -> Result<Node, LogicError> {
    use LogicValueType::{Boolean, Number, Text};
    let answered = matches!(&expr, Expression::Answered { .. });
    let variable = matches!(&expr, Expression::Variable { .. });
    let all = matches!(
        &expr,
        Expression::All { .. } | Expression::ContainsAll { .. }
    );
    let multiple = matches!(
        &expr,
        Expression::ContainsAny { .. } | Expression::ContainsAll { .. }
    );
    let mismatch = || error(LogicErrorCode::TypeMismatch, &path, None);
    let mut child = |expr, field| {
        compile_node(expr, format!("{path}.{field}"), bindings, dependencies).map(Box::new)
    };
    let (kind, value_type) = match expr {
        Expression::Boolean { value } => (NodeKind::Literal(LogicValue::Boolean(value)), Boolean),
        Expression::Number { value } => {
            let value =
                numeric(&value).ok_or_else(|| error(LogicErrorCode::InvalidNumber, &path, None))?;
            (NodeKind::Literal(LogicValue::Number(value)), Number)
        }
        Expression::Text { value } => {
            if value.contains('\0') {
                return Err(error(LogicErrorCode::InvalidSyntax, &path, None));
            }
            if value.chars().count() > MAX_TEXT_LENGTH {
                return Err(error(LogicErrorCode::LimitExceeded, &path, None));
            }
            (NodeKind::Literal(LogicValue::Text(value)), Text)
        }
        Expression::Answer { question_id }
        | Expression::Answered { question_id }
        | Expression::Variable {
            variable_id: question_id,
        } => {
            let id = reference(&question_id, &path)?;
            let binding = bindings
                .get(&id)
                .ok_or_else(|| error(LogicErrorCode::InvalidReference, &path, Some(id)))?;
            if binding.variable != variable {
                return Err(error(
                    LogicErrorCode::InvalidReference,
                    &path,
                    if variable { None } else { Some(id) },
                ));
            }
            dependencies.insert(id);
            if answered {
                (NodeKind::Answered(id), Boolean)
            } else {
                (NodeKind::Answer(id), binding.value_type)
            }
        }
        Expression::Selected {
            question_id,
            option_id,
        } => {
            let (id, options) = selection(&question_id, vec![option_id], false, &path, bindings)?;
            dependencies.insert(id);
            (NodeKind::Selected(id, options, false), Boolean)
        }
        Expression::ContainsAny {
            question_id,
            option_ids,
        }
        | Expression::ContainsAll {
            question_id,
            option_ids,
        } => {
            let (id, options) = selection(&question_id, option_ids, multiple, &path, bindings)?;
            dependencies.insert(id);
            (NodeKind::Selected(id, options, all), Boolean)
        }
        Expression::Not { expression } => {
            let expression = child(*expression, "expression")?;
            if expression.value_type != Boolean {
                return Err(mismatch());
            }
            (NodeKind::Not(expression), Boolean)
        }
        Expression::All { expressions } | Expression::Any { expressions } => {
            if expressions.is_empty() {
                return Err(error(LogicErrorCode::InvalidSyntax, &path, None));
            }
            let mut nodes = Vec::with_capacity(expressions.len());
            for (index, expr) in expressions.into_iter().enumerate() {
                let node = compile_node(
                    expr,
                    format!("{path}.expressions[{index}]"),
                    bindings,
                    dependencies,
                )?;
                if node.value_type != Boolean {
                    return Err(mismatch());
                }
                nodes.push(node);
            }
            (NodeKind::Logical(nodes, all), Boolean)
        }
        Expression::Compare {
            comparison,
            left,
            right,
        } => {
            let left = child(*left, "left")?;
            let right = child(*right, "right")?;
            if left.value_type != right.value_type
                || !matches!(left.value_type, Boolean | Number | Text)
                || (!matches!(comparison, Comparison::Eq | Comparison::Ne)
                    && left.value_type != Number)
            {
                return Err(mismatch());
            }
            (NodeKind::Compare(comparison, left, right), Boolean)
        }
        Expression::Coalesce { value, fallback } => {
            let value = child(*value, "value")?;
            let fallback = child(*fallback, "fallback")?;
            if value.value_type != fallback.value_type {
                return Err(mismatch());
            }
            let value_type = value.value_type;
            (NodeKind::Coalesce(value, fallback), value_type)
        }
        Expression::Length { value } => {
            let value = child(*value, "value")?;
            if !matches!(value.value_type, Text | LogicValueType::Choices) {
                return Err(mismatch());
            }
            (NodeKind::Length(value), Number)
        }
        Expression::Arithmetic {
            operator,
            left,
            right,
        } => {
            let left = child(*left, "left")?;
            let right = child(*right, "right")?;
            if left.value_type != Number || right.value_type != Number {
                return Err(mismatch());
            }
            (NodeKind::Arithmetic(operator, left, right), Number)
        }
    };
    Ok(Node {
        kind,
        value_type,
        path,
    })
}

fn selection(
    question_id: &str,
    options: Vec<String>,
    multiple: bool,
    path: &str,
    bindings: &BTreeMap<Uuid, Binding>,
) -> Result<(Uuid, Vec<Uuid>), LogicError> {
    let id = reference(question_id, path)?;
    let invalid = || error(LogicErrorCode::InvalidReference, path, Some(id));
    let binding = bindings.get(&id).ok_or_else(invalid)?;
    if binding.variable {
        return Err(invalid());
    }
    if binding.value_type != LogicValueType::Choices
        && (multiple || binding.value_type != LogicValueType::Choice)
    {
        return Err(error(LogicErrorCode::TypeMismatch, path, Some(id)));
    }
    if options.is_empty() || options.len() > MAX_CHOICES {
        return Err(error(LogicErrorCode::InvalidSyntax, path, Some(id)));
    }
    let mut ids = BTreeSet::new();
    for raw in options {
        let option = reference(&raw, path).map_err(|_| invalid())?;
        if !binding.options.contains(&option) || !ids.insert(option) {
            return Err(invalid());
        }
    }
    Ok((id, ids.into_iter().collect()))
}

fn read_answer(raw: Option<&Value>, binding: &Binding, id: Uuid) -> Result<LogicValue, LogicError> {
    // 仅在答案不合法时才格式化错误路径，避免成功热路径每次分配 String。
    let invalid = || {
        error(
            LogicErrorCode::InvalidAnswer,
            &format!("$.answers.{id}"),
            Some(id),
        )
    };
    let Some(raw) = raw.filter(|value| !value.is_null()) else {
        return Ok(LogicValue::Missing);
    };
    match binding.value_type {
        LogicValueType::Boolean => Err(invalid()),
        LogicValueType::Number => finite_number(raw, id)
            .map(canonical_zero)
            .map(LogicValue::Number)
            .map_err(|_| invalid()),
        LogicValueType::Text => raw
            .as_str()
            .map(|text| LogicValue::Text(text.into()))
            .ok_or_else(invalid),
        LogicValueType::Choice => {
            let option = raw
                .as_str()
                .and_then(crate::identity::parse)
                .filter(|option| binding.options.contains(option))
                .ok_or_else(invalid)?;
            Ok(LogicValue::Choice(option))
        }
        LogicValueType::Choices => {
            let values = raw
                .as_array()
                .filter(|values| values.len() <= MAX_CHOICES)
                .ok_or_else(invalid)?;
            let mut options = BTreeSet::new();
            for value in values {
                let option = value
                    .as_str()
                    .and_then(crate::identity::parse)
                    .filter(|option| binding.options.contains(option))
                    .ok_or_else(invalid)?;
                if !options.insert(option) {
                    return Err(invalid());
                }
            }
            Ok(LogicValue::Choices(options.into_iter().collect()))
        }
    }
}

impl Node {
    /// 求值时必然读取的题目；`all`/`any` 的后续分支不再读取已被存在性保护覆盖的题目。
    fn required_reads(
        &self,
        bindings: &BTreeMap<Uuid, Binding>,
        variables: &BTreeMap<Uuid, BTreeSet<Uuid>>,
    ) -> BTreeSet<Uuid> {
        match &self.kind {
            // 常量与 answered 只判断有无答案，不读取答案内容。
            NodeKind::Literal(_) | NodeKind::Answered(_) => BTreeSet::new(),
            NodeKind::Answer(id) => {
                if bindings[id].variable {
                    // 拓扑序保证依赖已展开；缺失说明编译器漏填，直接崩而不是掩盖。
                    variables[id].clone()
                } else {
                    [*id].into_iter().collect()
                }
            }
            NodeKind::Selected(id, ..) => [*id].into_iter().collect(),
            NodeKind::Not(expression) => expression.required_reads(bindings, variables),
            NodeKind::Logical(expressions, all) => {
                let mut reads = BTreeSet::new();
                let mut guarded = BTreeSet::new();
                for expression in expressions {
                    reads.extend(
                        expression
                            .required_reads(bindings, variables)
                            .difference(&guarded),
                    );
                    guarded.extend(expression.presence_guards(*all));
                }
                reads
            }
            // coalesce 在首值缺失时才会读取回退值。
            NodeKind::Coalesce(_, fallback) => fallback.required_reads(bindings, variables),
            NodeKind::Compare(_, left, right) | NodeKind::Arithmetic(_, left, right) => {
                let mut reads = left.required_reads(bindings, variables);
                reads.extend(right.required_reads(bindings, variables));
                reads
            }
            NodeKind::Length(value) => value.required_reads(bindings, variables),
        }
    }

    /// 在 `truth` 取该值时保证子表达式“有答案”的题目集合。
    fn presence_guards(&self, truth: bool) -> BTreeSet<Uuid> {
        match &self.kind {
            NodeKind::Answered(id) if truth => [*id].into_iter().collect(),
            NodeKind::Not(expression) => expression.presence_guards(!truth),
            NodeKind::Logical(expressions, all) if *all == truth => expressions
                .iter()
                .flat_map(|expression| expression.presence_guards(truth))
                .collect(),
            _ => BTreeSet::new(),
        }
    }

    fn evaluate(
        &self,
        context: &mut dyn FnMut(Uuid) -> Result<LogicValue, LogicError>,
    ) -> Result<LogicValue, LogicError> {
        use LogicValue::{Boolean, Missing, Number};
        let mismatch = || error(LogicErrorCode::TypeMismatch, &self.path, None);
        match &self.kind {
            NodeKind::Literal(value) => Ok(value.clone()),
            NodeKind::Answer(id) => context(*id),
            NodeKind::Answered(id) => Ok(Boolean(!matches!(context(*id)?, Missing))),
            NodeKind::Selected(id, expected, all) => match context(*id)? {
                Missing => Ok(Missing),
                LogicValue::Choice(option) => Ok(Boolean(expected.contains(&option))),
                LogicValue::Choices(options) => Ok(Boolean(if *all {
                    expected.iter().all(|id| options.contains(id))
                } else {
                    expected.iter().any(|id| options.contains(id))
                })),
                _ => Err(mismatch()),
            },
            NodeKind::Not(expr) => match expr.evaluate(context)? {
                Boolean(value) => Ok(Boolean(!value)),
                Missing => Ok(Missing),
                _ => Err(mismatch()),
            },
            NodeKind::Logical(expressions, all) => {
                let mut missing = false;
                for expr in expressions {
                    match expr.evaluate(context)? {
                        Boolean(value) if value != *all => return Ok(Boolean(value)),
                        Boolean(_) => {}
                        Missing => missing = true,
                        _ => return Err(mismatch()),
                    }
                }
                Ok(if missing { Missing } else { Boolean(*all) })
            }
            NodeKind::Compare(comparison, left, right) => {
                let left = left.evaluate(context)?;
                let right = right.evaluate(context)?;
                let order = match (&left, &right) {
                    (Missing, _) | (_, Missing) => return Ok(Missing),
                    (Number(a), Number(b)) => a.total_cmp(b),
                    (Boolean(a), Boolean(b)) => a.cmp(b),
                    (LogicValue::Text(a), LogicValue::Text(b)) => a.cmp(b),
                    _ => return Err(mismatch()),
                };
                Ok(Boolean(match comparison {
                    Comparison::Eq => order.is_eq(),
                    Comparison::Ne => !order.is_eq(),
                    Comparison::Lt => order.is_lt(),
                    Comparison::Le => order != Ordering::Greater,
                    Comparison::Gt => order.is_gt(),
                    Comparison::Ge => order != Ordering::Less,
                }))
            }
            NodeKind::Coalesce(value, fallback) => match value.evaluate(context)? {
                Missing => fallback.evaluate(context),
                value => Ok(value),
            },
            NodeKind::Length(value) => {
                let count = match value.evaluate(context)? {
                    Missing => return Ok(Missing),
                    LogicValue::Text(text) => text.chars().count(),
                    LogicValue::Choices(options) => options.len(),
                    _ => return Err(mismatch()),
                };
                let count = u32::try_from(count)
                    .map_err(|_| error(LogicErrorCode::LimitExceeded, &self.path, None))?;
                Ok(Number(f64::from(count)))
            }
            NodeKind::Arithmetic(operator, left, right) => {
                let left = left.evaluate(context)?;
                let right = right.evaluate(context)?;
                let (a, b) = match (left, right) {
                    (Missing, _) | (_, Missing) => return Ok(Missing),
                    (Number(a), Number(b)) => (a, b),
                    _ => return Err(mismatch()),
                };
                let value = match operator {
                    Arithmetic::Add => a + b,
                    Arithmetic::Subtract => a - b,
                    Arithmetic::Multiply => a * b,
                    Arithmetic::Divide => a / b,
                };
                if !value.is_finite() {
                    return Err(error(LogicErrorCode::ArithmeticError, &self.path, None));
                }
                Ok(Number(canonical_zero(value)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use serde_json::{Value, json};
    use uuid::Uuid;

    use super::*;
    use crate::domain::{Question, QuestionKind};

    fn id(value: u128) -> Uuid {
        Uuid::from_u128(value)
    }

    fn 数字题(value: u128) -> Question {
        Question {
            id: id(value),
            title: format!("数字题 {value}"),
            required: false,
            kind: QuestionKind::Number {
                integer: false,
                min: None,
                max: None,
            },
        }
    }

    fn 编译(expression: Value, questions: &[Question]) -> CompiledExpression {
        compile_expression(
            &json!({"dsl_version": 1, "semantics_version": 1, "expression": expression}),
            questions,
        )
        .expect("测试表达式应可编译")
    }

    #[test]
    fn 值类型名称拒绝未知名称() {
        for 未知 in ["", "bool", "Choice", "choices ", "number2"] {
            assert_eq!(LogicValueType::from_name(未知), None, "{未知}");
        }
    }

    #[test]
    fn 错误路径重定位只替换表达式前缀() {
        let 重定位 = error(
            LogicErrorCode::InvalidSyntax,
            "$.expression.left",
            Some(id(1)),
        )
        .relocate("$.logic.rules[0].visible_if");
        assert_eq!(重定位.path, "$.logic.rules[0].visible_if.left");

        // 根路径不含可替换前缀，必须保持原样。
        let 根 = error(LogicErrorCode::UnsupportedVersion, "$", None).relocate("$.logic.flow");
        assert_eq!(根.path, "$");
    }

    #[test]
    fn 数字常量只接受无空白_json_数字() {
        for (原文, 期望) in [
            ("0", 0.0),
            ("1", 1.0),
            ("1.5", 1.5),
            ("1e3", 1000.0),
            ("-2.5", -2.5),
        ] {
            assert_eq!(numeric(原文), Some(期望), "{原文}");
        }
        assert_eq!(
            numeric("-0").map(f64::to_bits),
            Some(0.0_f64.to_bits()),
            "负零归一为正零"
        );

        for 原文 in [
            " 1", "1 ", "+1", "01", ".5", "1.", "1e400", "NaN", "Infinity", "", "-",
        ] {
            assert_eq!(numeric(原文), None, "应拒绝 {原文:?}");
        }
    }

    #[test]
    fn 选择器校验题型选项归属与重复() {
        let mut bindings = BTreeMap::new();
        bindings.insert(
            id(1),
            Binding {
                value_type: LogicValueType::Choice,
                options: vec![id(101), id(102)],
                variable: false,
            },
        );
        bindings.insert(
            id(2),
            Binding {
                value_type: LogicValueType::Choices,
                options: vec![id(201)],
                variable: false,
            },
        );
        bindings.insert(
            id(3),
            Binding {
                value_type: LogicValueType::Text,
                options: Vec::new(),
                variable: false,
            },
        );
        bindings.insert(
            id(4),
            Binding {
                value_type: LogicValueType::Number,
                options: Vec::new(),
                variable: true,
            },
        );
        let 路径 = "$.expression";

        let 空 = selection(&id(1).to_string(), Vec::new(), false, 路径, &bindings).unwrap_err();
        assert_eq!(空.code, LogicErrorCode::InvalidSyntax);
        assert_eq!(空.question_id, Some(id(1)));
        assert_eq!(空.path, 路径);

        let 超量 = selection(
            &id(2).to_string(),
            vec![id(201).to_string(); MAX_CHOICES + 1],
            true,
            路径,
            &bindings,
        )
        .unwrap_err();
        assert_eq!(超量.code, LogicErrorCode::InvalidSyntax);
        assert_eq!(超量.question_id, Some(id(2)));

        let 重复 = selection(
            &id(2).to_string(),
            vec![id(201).to_string(); 2],
            true,
            路径,
            &bindings,
        )
        .unwrap_err();
        assert_eq!(重复.code, LogicErrorCode::InvalidReference);
        assert_eq!(重复.question_id, Some(id(2)));

        let 非选择 = selection(
            &id(3).to_string(),
            vec![id(101).to_string()],
            false,
            路径,
            &bindings,
        )
        .unwrap_err();
        assert_eq!(非选择.code, LogicErrorCode::TypeMismatch);
        assert_eq!(非选择.question_id, Some(id(3)));

        let 跨题 = selection(
            &id(1).to_string(),
            vec![id(201).to_string()],
            false,
            路径,
            &bindings,
        )
        .unwrap_err();
        assert_eq!(跨题.code, LogicErrorCode::InvalidReference);
        assert_eq!(跨题.question_id, Some(id(1)));

        let 未知 = selection(
            &id(999).to_string(),
            vec![id(101).to_string()],
            false,
            路径,
            &bindings,
        )
        .unwrap_err();
        assert_eq!(未知.code, LogicErrorCode::InvalidReference);
        assert_eq!(未知.question_id, Some(id(999)));

        let 变量 = selection(
            &id(4).to_string(),
            vec![id(101).to_string()],
            false,
            路径,
            &bindings,
        )
        .unwrap_err();
        assert_eq!(变量.code, LogicErrorCode::InvalidReference);
        assert_eq!(变量.question_id, Some(id(4)));

        assert_eq!(
            selection(
                &id(2).to_string(),
                vec![id(201).to_string()],
                true,
                路径,
                &bindings
            )
            .unwrap(),
            (id(2), vec![id(201)])
        );
    }

    #[test]
    fn 答案读取拒绝越界与超量选项() {
        let 单选 = Binding {
            value_type: LogicValueType::Choice,
            options: vec![id(101)],
            variable: false,
        };
        for 值 in [json!(id(999).to_string()), json!("not-a-uuid"), json!(101)] {
            let 错误 = read_answer(Some(&值), &单选, id(4)).unwrap_err();
            assert_eq!(错误.code, LogicErrorCode::InvalidAnswer, "{值}");
            assert_eq!(错误.question_id, Some(id(4)));
        }

        let 多选 = Binding {
            value_type: LogicValueType::Choices,
            options: vec![id(101), id(102), id(103)],
            variable: false,
        };
        let 超量 = read_answer(
            Some(&json!(vec![id(101).to_string(); MAX_CHOICES + 1])),
            &多选,
            id(5),
        )
        .unwrap_err();
        assert_eq!(超量.code, LogicErrorCode::InvalidAnswer);
    }

    #[test]
    fn 存在性守护决定必然读取的题目集合() {
        let questions = [数字题(1), 数字题(2)];
        let 答 =
            |question: u128| json!({"op": "answered", "question_id": id(question).to_string()});
        let 比较 = |question: u128| {
            json!({
                "op": "compare",
                "comparison": "eq",
                "left": {"op": "answer", "question_id": id(question).to_string()},
                "right": {"op": "number", "value": "1"},
            })
        };

        // answered 只判断有无答案，本身不读取答案内容。
        let 表达式 = 编译(答(1), &questions);
        assert!(表达式.required_reads(&BTreeMap::new()).is_empty());

        // 常量字面量没有任何题目依赖与存在性守护。
        let 常量 = 编译(json!({"op": "boolean", "value": true}), &questions);
        assert!(常量.required_reads(&BTreeMap::new()).is_empty());

        // all 中 answered(q1) 守护同一题的 answer 读取。
        let 守护 = 编译(
            json!({"op": "all", "expressions": [答(1), 比较(1)]}),
            &questions,
        );
        assert!(
            守护.required_reads(&BTreeMap::new()).is_empty(),
            "已由 answered(q1) 守护的读取不再计入必然读取"
        );
        let 未守护 = 编译(
            json!({"op": "all", "expressions": [答(1), 比较(2)]}),
            &questions,
        );
        assert_eq!(
            未守护.required_reads(&BTreeMap::new()),
            BTreeSet::from([id(2)])
        );

        // any 不能保证存在性，后续读取仍保留。
        let 表达式 = 编译(
            json!({"op": "any", "expressions": [答(1), 比较(1)]}),
            &questions,
        );
        assert_eq!(
            表达式.required_reads(&BTreeMap::new()),
            BTreeSet::from([id(1)])
        );

        // not(answered(q)) 不提供守护，后续读取仍保留。
        let 组合 = 编译(
            json!({"op": "all", "expressions": [json!({"op": "not", "expression": 答(1)}), 比较(1)]}),
            &questions,
        );
        assert_eq!(
            组合.required_reads(&BTreeMap::new()),
            BTreeSet::from([id(1)]),
            "否定存在性不提供守护"
        );

        // coalesce 只把回退分支视为必然读取。
        let 短路 = 编译(
            json!({"op": "coalesce", "value": {"op": "answer", "question_id": id(1).to_string()}, "fallback": {"op": "number", "value": "0"}}),
            &questions,
        );
        assert!(短路.required_reads(&BTreeMap::new()).is_empty());
        let 回退 = 编译(
            json!({"op": "coalesce", "value": {"op": "number", "value": "0"}, "fallback": {"op": "answer", "question_id": id(2).to_string()}}),
            &questions,
        );
        assert_eq!(
            回退.required_reads(&BTreeMap::new()),
            BTreeSet::from([id(2)])
        );
    }
}
