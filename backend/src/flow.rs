//! 流程只能向前转换；改答按前置有效答案重算路径，回退不恢复已清除的答案。

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::domain::{Answer, Document, Question, QuestionKind, canonical_number};
use crate::logic::{
    CompiledExpression, LogicError, LogicErrorCode, LogicValue, LogicValueType,
    compile_with_variables, error,
};
use crate::logic_document::{CompiledDocumentLogic, DocumentLogic, rewrite};

/// 节点数组采用拓扑顺序，入口必须是第一个页面。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FlowDefinition {
    /// 入口页面的稳定身份。
    pub entry: Uuid,
    /// 页面、自动分支和结束节点。
    pub nodes: Vec<FlowNode>,
}

/// 按声明顺序匹配；未知条件阻止选择更低优先级的分支。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FlowBranch {
    /// 布尔表达式。
    pub when: Value,
    /// 后续节点身份。
    pub target: Uuid,
}

/// 结束仅代表流程结果，不能自动保存答卷或消耗账号额度。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FlowOutcome {
    /// 等待用户确认提交。
    Complete,
    /// 筛选结束，暂不产生持久化副作用。
    ScreenOut,
}

/// 流程节点使用封闭枚举，避免页面与结束状态同时成立。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum FlowNode {
    /// 一页有序题目，点击下一步后才离开。
    Page {
        /// 页面身份。
        id: Uuid,
        /// 题目身份；全流程恰好覆盖文档题目一次。
        question_ids: Vec<Uuid>,
        /// 下一节点。
        next: Uuid,
    },
    /// 自动条件分支，必须有默认出口。
    Branch {
        /// 分支节点身份。
        id: Uuid,
        /// 有优先级的条件列表。
        branches: Vec<FlowBranch>,
        /// 所有条件均为 false 时的出口。
        otherwise: Uuid,
    },
    /// 终点。
    End {
        /// 终点身份。
        id: Uuid,
        /// 结果种类。
        outcome: FlowOutcome,
    },
}

impl FlowNode {
    fn id(&self) -> Uuid {
        match self {
            Self::Page { id, .. } | Self::Branch { id, .. } | Self::End { id, .. } => *id,
        }
    }
    fn targets(&self) -> Vec<Uuid> {
        match self {
            Self::Page { next, .. } => vec![*next],
            Self::Branch {
                branches,
                otherwise,
                ..
            } => branches
                .iter()
                .map(|branch| branch.target)
                .chain([*otherwise])
                .collect(),
            Self::End { .. } => Vec::new(),
        }
    }
}

impl FlowDefinition {
    pub(crate) fn expressions(&self) -> impl Iterator<Item = &Value> {
        self.nodes
            .iter()
            .flat_map(|node| match node {
                FlowNode::Branch { branches, .. } => branches.as_slice(),
                _ => &[],
            })
            .map(|branch| &branch.when)
    }

    pub(crate) fn renew_ids(
        &mut self,
        questions: &BTreeMap<Uuid, Uuid>,
        options: &BTreeMap<Uuid, Uuid>,
        variables: &BTreeMap<Uuid, Uuid>,
    ) {
        let nodes: BTreeMap<_, _> = self
            .nodes
            .iter()
            .map(|node| (node.id(), Uuid::new_v4()))
            .collect();
        let remap = |id: &mut Uuid| {
            if let Some(next) = nodes.get(id) {
                *id = *next;
            }
        };
        remap(&mut self.entry);
        for node in &mut self.nodes {
            match node {
                FlowNode::Page {
                    id,
                    question_ids,
                    next,
                } => {
                    remap(id);
                    remap(next);
                    for id in question_ids {
                        if let Some(next) = questions.get(id) {
                            *id = *next;
                        }
                    }
                }
                FlowNode::Branch {
                    id,
                    branches,
                    otherwise,
                } => {
                    remap(id);
                    remap(otherwise);
                    for branch in branches {
                        remap(&mut branch.target);
                        rewrite(&mut branch.when, questions, options, variables);
                    }
                }
                FlowNode::End { id, .. } => remap(id),
            }
        }
    }
}

#[derive(Debug)]
enum Node {
    Page {
        id: Uuid,
        questions: Vec<Uuid>,
        next: usize,
    },
    Branch {
        guards: Vec<(CompiledExpression, usize)>,
        otherwise: usize,
    },
    End {
        id: Uuid,
        outcome: FlowOutcome,
    },
}

#[derive(Debug)]
pub(crate) struct CompiledFlow {
    nodes: Vec<Node>,
}

impl CompiledFlow {
    fn implicit(questions: &[Question]) -> Self {
        // 虚拟身份只存在于运行状态，不写入旧文档或改变版本指纹。
        Self {
            nodes: vec![
                Node::Page {
                    id: Uuid::nil(),
                    questions: questions.iter().map(|question| question.id).collect(),
                    next: 1,
                },
                Node::End {
                    id: Uuid::from_u128(1),
                    outcome: FlowOutcome::Complete,
                },
            ],
        }
    }
    fn id(&self, index: usize) -> Uuid {
        match &self.nodes[index] {
            Node::Page { id, .. } | Node::End { id, .. } => *id,
            Node::Branch { .. } => Uuid::nil(),
        }
    }
}

pub(crate) fn compile_flow(
    flow: &FlowDefinition,
    definition: &DocumentLogic,
    logic: &CompiledDocumentLogic,
    questions: &[Question],
) -> Result<CompiledFlow, LogicError> {
    let invalid = || error(LogicErrorCode::InvalidFlow, "$.logic.flow", None);
    if flow.nodes.len() > 256 {
        return Err(error(LogicErrorCode::LimitExceeded, "$.logic.flow", None));
    }
    if !matches!(flow.nodes.first(), Some(FlowNode::Page { id, .. }) if *id == flow.entry) {
        return Err(invalid());
    }
    let mut occupied: BTreeSet<_> = questions
        .iter()
        .map(|question| question.id)
        .chain(definition.variables.iter().map(|variable| variable.id))
        .collect();
    for question in questions {
        if let QuestionKind::SingleChoice { options }
        | QuestionKind::MultipleChoice { options, .. } = &question.kind
        {
            occupied.extend(options.iter().map(|option| option.id));
        }
    }
    let mut indices = BTreeMap::new();
    for (index, node) in flow.nodes.iter().enumerate() {
        if node.id().is_nil() || !occupied.insert(node.id()) {
            return Err(invalid());
        }
        indices.insert(node.id(), index);
    }
    let mut flattened = Vec::new();
    let mut pages = BTreeMap::new();
    let mut predecessors = vec![Vec::new(); flow.nodes.len()];
    for (index, node) in flow.nodes.iter().enumerate() {
        if let FlowNode::Page { question_ids, .. } = node {
            if question_ids.is_empty() {
                return Err(invalid());
            }
            for id in question_ids {
                flattened.push(*id);
                pages.insert(*id, index);
            }
        }
        if let FlowNode::Branch { branches, .. } = node
            && (branches.is_empty() || branches.len() > 16)
        {
            return Err(invalid());
        }
        for target in node.targets() {
            let next = indices.get(&target).ok_or_else(invalid)?;
            if *next <= index {
                return Err(invalid());
            }
            predecessors[*next].push(index);
        }
    }
    if flattened
        != questions
            .iter()
            .map(|question| question.id)
            .collect::<Vec<_>>()
    {
        return Err(invalid());
    }
    let mut dominators: Vec<BTreeSet<usize>> = Vec::with_capacity(flow.nodes.len());
    for (index, parents) in predecessors.iter().enumerate() {
        let mut common = if index == 0 {
            BTreeSet::new()
        } else {
            let Some(first) = parents.first() else {
                return Err(invalid());
            };
            dominators[*first].clone()
        };
        for parent in parents {
            common.retain(|node| dominators[*parent].contains(node));
        }
        common.insert(index);
        dominators.push(common);
    }
    let variable_types = definition
        .variables
        .iter()
        .map(|variable| (variable.id, variable.value_type))
        .collect();
    let raw_variables: BTreeMap<_, _> = definition
        .variables
        .iter()
        .map(|variable| (variable.id, &variable.expression))
        .collect();
    let mut variable_reads = BTreeMap::new();
    for id in logic.variable_order() {
        variable_reads.insert(*id, required_reads(raw_variables[id], &variable_reads));
    }
    let check_available = |raw: &Value, node: usize, path: &str| -> Result<(), LogicError> {
        if required_reads(raw, &variable_reads)
            .iter()
            .any(|id| !dominators[node].contains(&pages[id]))
        {
            return Err(error(LogicErrorCode::UnavailableDependency, path, None));
        }
        Ok(())
    };
    for (index, rule) in definition.rules.iter().enumerate() {
        for (field, raw) in [
            ("visible_if", &rule.visible_if),
            ("required_if", &rule.required_if),
            ("validate", &rule.validate),
        ] {
            if let Some(raw) = raw {
                check_available(
                    raw,
                    pages[&rule.question_id],
                    &format!("$.logic.rules[{index}].{field}"),
                )?;
            }
        }
    }
    let mut nodes = Vec::with_capacity(flow.nodes.len());
    for (index, node) in flow.nodes.iter().enumerate() {
        let compiled = match node {
            FlowNode::Page {
                id,
                question_ids,
                next,
            } => Node::Page {
                id: *id,
                questions: question_ids.clone(),
                next: indices[next],
            },
            FlowNode::End { id, outcome } => Node::End {
                id: *id,
                outcome: *outcome,
            },
            FlowNode::Branch {
                branches,
                otherwise,
                ..
            } => {
                let mut guards = Vec::with_capacity(branches.len());
                for (branch_index, branch) in branches.iter().enumerate() {
                    let path = format!("$.logic.flow.nodes[{index}].branches[{branch_index}].when");
                    let program = json!({"dsl_version": definition.dsl_version, "semantics_version": definition.semantics_version, "expression": branch.when});
                    let expression = compile_with_variables(&program, questions, &variable_types)
                        .map_err(|mut failure| {
                        failure.path = failure.path.replacen("$.expression", &path, 1);
                        failure
                    })?;
                    if expression.value_type() != LogicValueType::Boolean {
                        return Err(error(LogicErrorCode::TypeMismatch, &path, None));
                    }
                    if branch.when["op"] == "boolean"
                        && (branch.when["value"] == false || branch_index + 1 < branches.len())
                    {
                        return Err(error(LogicErrorCode::InvalidFlow, &path, None));
                    }
                    if logic
                        .dependencies(&expression)
                        .iter()
                        .any(|id| pages[id] >= index)
                    {
                        return Err(error(LogicErrorCode::ForwardReference, &path, None));
                    }
                    check_available(&branch.when, index, &path)?;
                    guards.push((expression, indices[&branch.target]));
                }
                Node::Branch {
                    guards,
                    otherwise: indices[otherwise],
                }
            }
        };
        nodes.push(compiled);
    }
    Ok(CompiledFlow { nodes })
}

// 对已经完成类型检查的 AST 做保守缺失分析；不展开变量 AST，避免指数膨胀。
fn required_reads(raw: &Value, variables: &BTreeMap<Uuid, BTreeSet<Uuid>>) -> BTreeSet<Uuid> {
    let uuid = |key| {
        raw.get(key)
            .and_then(Value::as_str)
            .and_then(|id| Uuid::parse_str(id).ok())
    };
    match raw.get("op").and_then(Value::as_str).unwrap_or("") {
        "answer" | "selected" | "contains_any" | "contains_all" => {
            uuid("question_id").into_iter().collect()
        }
        "variable" => uuid("variable_id")
            .and_then(|id| variables.get(&id))
            .cloned()
            .unwrap_or_default(),
        "answered" => BTreeSet::new(),
        "coalesce" => required_reads(&raw["fallback"], variables),
        "all" | "any" => {
            let mut reads = BTreeSet::new();
            let mut guarded = BTreeSet::new();
            if let Some(expressions) = raw["expressions"].as_array() {
                for expression in expressions {
                    reads.extend(required_reads(expression, variables).difference(&guarded));
                    guarded.extend(presence_guards(expression, raw["op"] == "all"));
                }
            }
            reads
        }
        _ => ["expression", "left", "right", "value"]
            .into_iter()
            .filter_map(|key| raw.get(key))
            .flat_map(|child| required_reads(child, variables))
            .collect(),
    }
}

fn presence_guards(raw: &Value, truth: bool) -> BTreeSet<Uuid> {
    match raw.get("op").and_then(Value::as_str).unwrap_or("") {
        "answered" if truth => raw["question_id"]
            .as_str()
            .and_then(|id| Uuid::parse_str(id).ok())
            .into_iter()
            .collect(),
        "not" => presence_guards(&raw["expression"], !truth),
        "all" if truth => raw["expressions"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|child| presence_guards(child, true))
            .collect(),
        "any" if !truth => raw["expressions"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|child| presence_guards(child, false))
            .collect(),
        _ => BTreeSet::new(),
    }
}

/// 一个外部事件完整处理后才更新状态；答案使用与提交相同的 JSON 类型。
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum FlowEvent {
    /// 修改当前页面上的可见题目。
    Change {
        /// 题目身份。
        question_id: Uuid,
        /// 原始 JSON 答案，null 表示清空。
        value: Value,
    },
    /// 校验当前页面后前进。
    Next,
    /// 回到当前有效历史的上一页。
    Previous,
}

#[derive(Debug, Default)]
struct Projection {
    path: Vec<usize>,
    visible: Vec<Uuid>,
    answers: BTreeMap<Uuid, Value>,
    errors: BTreeMap<Uuid, LogicErrorCode>,
    route_error: Option<LogicErrorCode>,
}

/// 对外只读快照；当前页与预测路径分开，结束不代表已提交。
#[derive(Debug, Serialize)]
pub struct FlowSnapshot {
    /// 当前页面或结束节点。
    pub current_node: Uuid,
    /// 只有真正前进到结束节点后才有结果。
    pub outcome: Option<FlowOutcome>,
    /// 实际访问且仍有效的历史。
    pub history: Vec<Uuid>,
    /// 根据当前答案推导出的页面及结束节点路径。
    pub path: Vec<Uuid>,
    /// 当前路径上的可见题目。
    pub visible: Vec<Uuid>,
    /// 已校验的有效答案。
    pub answers: BTreeMap<Uuid, Value>,
    /// 可见题目的错误；隐藏题目不留错误。
    pub errors: BTreeMap<Uuid, LogicErrorCode>,
    /// 路径末端的未决分支或计算错误。
    pub route_error: Option<LogicErrorCode>,
    /// 最近一次改答清除的题目身份。
    pub cleared: Vec<Uuid>,
}

/// 某题在最终有效路径上的适用性；跳过与可选未答不可混用。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnswerState {
    /// 适用且已有有效答案。
    Answered,
    /// 适用但未回答。
    Unanswered,
    /// 被显示规则或流程分支跳过。
    Skipped,
}

/// 已到达 complete 的重建结果，不表示数据库已经保存答卷。
#[derive(Debug, Serialize)]
pub struct EvaluatedResponse {
    /// 用于存储与幂等比较的规范化答案。
    pub answers: BTreeMap<Uuid, Value>,
    /// 覆盖全部题目的适用性。
    pub question_states: BTreeMap<Uuid, AnswerState>,
    /// 从最终答案重建的路径，不是实际访问轨迹。
    pub path: Vec<Uuid>,
}

/// 只持有编译结果与不可变题面；交互会话和服务端重建共用一次顺序求值。
#[derive(Debug)]
pub struct FlowEvaluator<'a> {
    document: &'a Document,
    logic: CompiledDocumentLogic,
    flow: CompiledFlow,
}

impl<'a> FlowEvaluator<'a> {
    /// 编译并绑定文档；同一发布版本可复用此实例重建多份答卷。
    ///
    /// # 错误
    /// 草稿结构、规则或流程不合法时返回定位错误。
    pub fn new(document: &'a Document) -> Result<Self, LogicError> {
        let empty = DocumentLogic {
            flow: None,
            dsl_version: 1.into(),
            semantics_version: 1.into(),
            variables: Vec::new(),
            rules: Vec::new(),
        };
        let mut logic = document
            .logic
            .as_ref()
            .unwrap_or(&empty)
            .compile(&document.questions)?;
        document
            .validate_copy_draft()
            .map_err(|failure| error(LogicErrorCode::InvalidFlow, "$", failure.question_id))?;
        let flow = logic
            .flow
            .take()
            .unwrap_or_else(|| CompiledFlow::implicit(&document.questions));
        Ok(Self {
            document,
            logic,
            flow,
        })
    }

    /// 从入口重建最终答卷，不接受客户端历史或预先计算的可见题目。
    ///
    /// # 错误
    /// 重复/未知题目、不适用答案、校验失败、未决条件或筛选结束均拒绝。
    pub fn evaluate_answers(&self, answers: Vec<Answer>) -> Result<EvaluatedResponse, LogicError> {
        let mut inputs = BTreeMap::new();
        for answer in answers {
            if !self
                .document
                .questions
                .iter()
                .any(|question| question.id == answer.question_id)
            {
                return Err(error(
                    LogicErrorCode::InvalidReference,
                    "$.answers",
                    Some(answer.question_id),
                ));
            }
            if inputs.insert(answer.question_id, answer.value).is_some() {
                return Err(error(
                    LogicErrorCode::InvalidAnswer,
                    "$.answers",
                    Some(answer.question_id),
                ));
            }
        }
        self.finish(&inputs)
    }

    // 只用于数据库中的已规范化答案。JSONB 会展开指数，不能再次套用原始传输字节预算
    // 或裸整数词法限制；恢复 f64 的规范表示后，仍完整重算规则、范围和路径。
    pub(crate) fn evaluate_stored(
        &self,
        mut answers: BTreeMap<Uuid, Value>,
    ) -> Result<EvaluatedResponse, LogicError> {
        for question in &self.document.questions {
            if matches!(
                question.kind,
                QuestionKind::Number { .. } | QuestionKind::Rating { .. }
            ) && let Some(value) = answers.get_mut(&question.id)
            {
                let number = value
                    .as_f64()
                    .filter(|number| number.is_finite())
                    .ok_or_else(|| {
                        error(
                            LogicErrorCode::InvalidAnswer,
                            "$.answers",
                            Some(question.id),
                        )
                    })?;
                *value = canonical_number(number, question.id).map_err(|_| {
                    error(
                        LogicErrorCode::InvalidAnswer,
                        "$.answers",
                        Some(question.id),
                    )
                })?;
            }
        }
        self.finish(&answers)
    }

    fn finish(&self, inputs: &BTreeMap<Uuid, Value>) -> Result<EvaluatedResponse, LogicError> {
        let projection = self.project(inputs);
        for question in &self.document.questions {
            if let Some(code) = projection.errors.get(&question.id) {
                return Err(error(*code, "$.answers", Some(question.id)));
            }
        }
        if let Some(code) = projection.route_error {
            return Err(error(code, "$.route", None));
        }
        for id in inputs.keys() {
            if !projection.visible.contains(id) {
                return Err(error(
                    LogicErrorCode::InapplicableAnswer,
                    "$.answers",
                    Some(*id),
                ));
            }
        }
        match projection.path.last().map(|index| &self.flow.nodes[*index]) {
            Some(Node::End {
                outcome: FlowOutcome::Complete,
                ..
            }) => {}
            Some(Node::End {
                outcome: FlowOutcome::ScreenOut,
                ..
            }) => return Err(error(LogicErrorCode::ScreenedOut, "$.route", None)),
            _ => return Err(error(LogicErrorCode::UnknownCondition, "$.route", None)),
        }
        let question_states = self
            .document
            .questions
            .iter()
            .map(|question| {
                let state = if !projection.visible.contains(&question.id) {
                    AnswerState::Skipped
                } else if projection.answers.contains_key(&question.id) {
                    AnswerState::Answered
                } else {
                    AnswerState::Unanswered
                };
                (question.id, state)
            })
            .collect();
        Ok(EvaluatedResponse {
            answers: projection.answers,
            question_states,
            path: projection
                .path
                .iter()
                .map(|index| self.flow.id(*index))
                .collect(),
        })
    }

    fn project(&self, inputs: &BTreeMap<Uuid, Value>) -> Projection {
        let mut result = Projection::default();
        let mut current = 0;
        loop {
            match &self.flow.nodes[current] {
                Node::Page {
                    questions, next, ..
                } => {
                    result.path.push(current);
                    for id in questions {
                        let Some(question) = self
                            .document
                            .questions
                            .iter()
                            .find(|question| question.id == *id)
                        else {
                            continue;
                        };
                        let rule = self.logic.rule(*id);
                        let eval = |expression: Option<&CompiledExpression>,
                                    answers: &BTreeMap<Uuid, Value>,
                                    default| {
                            expression
                                .map(|expression| {
                                    self.logic.evaluate_expression(expression, answers)
                                })
                                .unwrap_or(Ok(LogicValue::Boolean(default)))
                        };
                        match eval(
                            rule.and_then(|rule| rule.visible_if.as_ref()),
                            &result.answers,
                            true,
                        ) {
                            Ok(LogicValue::Boolean(true)) => {}
                            Ok(_) => continue,
                            Err(failure) => {
                                result.visible.push(*id);
                                result.errors.insert(*id, failure.code);
                                continue;
                            }
                        }
                        result.visible.push(*id);
                        let value = match question
                            .normalize_answer(inputs.get(id).cloned().unwrap_or(Value::Null))
                        {
                            Ok(value) => value,
                            Err(_) => {
                                result.errors.insert(*id, LogicErrorCode::InvalidAnswer);
                                continue;
                            }
                        };
                        let required = if question.required {
                            Ok(LogicValue::Boolean(true))
                        } else {
                            eval(
                                rule.and_then(|rule| rule.required_if.as_ref()),
                                &result.answers,
                                false,
                            )
                        };
                        match required {
                            Ok(LogicValue::Boolean(true)) if value.is_none() => {
                                result.errors.insert(*id, LogicErrorCode::RequiredAnswer);
                            }
                            Ok(LogicValue::Missing) => {
                                result.errors.insert(*id, LogicErrorCode::UnknownCondition);
                            }
                            Err(failure) => {
                                result.errors.insert(*id, failure.code);
                            }
                            _ => {}
                        }
                        if let Some(value) = value {
                            result.answers.insert(*id, value);
                            match eval(
                                rule.and_then(|rule| rule.validate.as_ref()),
                                &result.answers,
                                true,
                            ) {
                                Ok(LogicValue::Boolean(true)) => {}
                                Ok(LogicValue::Missing) => {
                                    result.errors.insert(*id, LogicErrorCode::UnknownCondition);
                                }
                                Ok(_) => {
                                    result.errors.insert(*id, LogicErrorCode::ConstraintFailed);
                                }
                                Err(failure) => {
                                    result.errors.insert(*id, failure.code);
                                }
                            }
                            if result.errors.contains_key(id) {
                                result.answers.remove(id);
                            }
                        }
                    }
                    current = *next;
                }
                Node::Branch { guards, otherwise } => {
                    let mut target = *otherwise;
                    for (guard, next) in guards {
                        match self.logic.evaluate_expression(guard, &result.answers) {
                            Ok(LogicValue::Boolean(true)) => {
                                target = *next;
                                break;
                            }
                            Ok(LogicValue::Boolean(false)) => {}
                            Ok(_) => {
                                result.route_error = Some(LogicErrorCode::UnknownCondition);
                                return result;
                            }
                            Err(failure) => {
                                result.route_error = Some(failure.code);
                                return result;
                            }
                        }
                    }
                    current = target;
                }
                Node::End { .. } => {
                    result.path.push(current);
                    return result;
                }
            }
        }
    }
}

/// 状态只能通过事件修改，不能从客户端反序列化可信运行历史。
#[derive(Debug)]
pub struct FlowSession<'a> {
    evaluator: FlowEvaluator<'a>,
    inputs: BTreeMap<Uuid, Value>,
    history: Vec<usize>,
    projection: Projection,
    cleared: Vec<Uuid>,
}

impl<'a> FlowSession<'a> {
    /// 绑定不可变文档并创建空填写会话；旧问卷使用内存中的单页流程。
    ///
    /// # 错误
    /// 草稿、规则或流程不合法时拒绝启动。
    pub fn new(document: &'a Document) -> Result<Self, LogicError> {
        let evaluator = FlowEvaluator::new(document)?;
        let projection = evaluator.project(&BTreeMap::new());
        Ok(Self {
            evaluator,
            inputs: BTreeMap::new(),
            history: vec![0],
            projection,
            cleared: Vec::new(),
        })
    }

    /// 返回独立的只读快照，外部修改不会改变会话。
    #[must_use]
    pub fn snapshot(&self) -> FlowSnapshot {
        let current = self.current();
        FlowSnapshot {
            current_node: self.evaluator.flow.id(current),
            outcome: match &self.evaluator.flow.nodes[current] {
                Node::End { outcome, .. } => Some(*outcome),
                _ => None,
            },
            history: self
                .history
                .iter()
                .map(|index| self.evaluator.flow.id(*index))
                .collect(),
            path: self
                .projection
                .path
                .iter()
                .map(|index| self.evaluator.flow.id(*index))
                .collect(),
            visible: self.projection.visible.clone(),
            answers: self.projection.answers.clone(),
            errors: self.projection.errors.clone(),
            route_error: self.projection.route_error,
            cleared: self.cleared.clone(),
        }
    }

    fn current(&self) -> usize {
        self.history.last().copied().unwrap_or(0)
    }

    /// 应用事件；不合法事件不修改状态，改答会保留输入错误供修正。
    ///
    /// # 错误
    /// 页面校验失败、条件未决或越过当前页面改答时返回定位错误。
    pub fn dispatch(&mut self, event: FlowEvent) -> Result<(), LogicError> {
        let invalid = || error(LogicErrorCode::InvalidEvent, "$.event", None);
        match event {
            FlowEvent::Change { question_id, value } => {
                let Node::Page { questions, .. } = &self.evaluator.flow.nodes[self.current()]
                else {
                    return Err(invalid());
                };
                if !questions.contains(&question_id)
                    || !self.projection.visible.contains(&question_id)
                {
                    return Err(invalid());
                }
                let mut inputs = self.inputs.clone();
                inputs.insert(question_id, value);
                let projection = self.evaluator.project(&inputs);
                self.cleared = inputs
                    .keys()
                    .filter(|id| !projection.visible.contains(id))
                    .copied()
                    .collect();
                inputs.retain(|id, _| projection.visible.contains(id));
                self.history.retain(|index| projection.path.contains(index));
                self.inputs = inputs;
                self.projection = projection;
            }
            FlowEvent::Previous => {
                if self.history.len() <= 1 {
                    return Err(invalid());
                }
                self.history.pop();
                self.cleared.clear();
            }
            FlowEvent::Next => {
                let current = self.current();
                let Node::Page { questions, .. } = &self.evaluator.flow.nodes[current] else {
                    return Err(invalid());
                };
                for id in questions {
                    if let Some(code) = self.projection.errors.get(id) {
                        return Err(error(*code, "$.answers", Some(*id)));
                    }
                }
                let Some(position) = self
                    .projection
                    .path
                    .iter()
                    .position(|index| *index == current)
                else {
                    return Err(invalid());
                };
                let next = self
                    .projection
                    .path
                    .iter()
                    .skip(position + 1)
                    .find(|index| match &self.evaluator.flow.nodes[**index] {
                        Node::Page { questions, .. } => questions
                            .iter()
                            .any(|id| self.projection.visible.contains(id)),
                        Node::End { .. } => true,
                        Node::Branch { .. } => false,
                    })
                    .copied();
                let Some(next) = next else {
                    return Err(error(
                        self.projection
                            .route_error
                            .unwrap_or(LogicErrorCode::UnknownCondition),
                        "$.route",
                        None,
                    ));
                };
                self.history.push(next);
                self.cleared.clear();
            }
        }
        Ok(())
    }
}
