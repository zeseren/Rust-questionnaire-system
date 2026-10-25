//! 流程只能向前转换；改答按前置有效答案重算路径，回退不恢复已清除的答案。

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::domain::{Answer, Document, Question};
use crate::logic::{
    CompilationContext, CompiledExpression, LogicError, LogicErrorCode, LogicValue, LogicValueType,
    error,
};
use crate::logic_document::{CompiledDocumentLogic, rewrite};

/// 流程节点总数上限。
const MAX_FLOW_NODES: usize = 256;
/// 单个分支节点的条件数上限。
const MAX_BRANCHES: usize = 16;
/// 未配置流程时虚拟入口页面的身份。
const IMPLICIT_PAGE_ID: Uuid = Uuid::nil();
/// 未配置流程时虚拟结束节点的身份。
const IMPLICIT_END_ID: Uuid = Uuid::from_u128(1);

/// 节点数组采用拓扑顺序，入口必须是第一个页面。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct FlowDefinition {
    /// 入口页面的稳定身份。
    #[serde(deserialize_with = "crate::identity::deserialize")]
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
    #[serde(deserialize_with = "crate::identity::deserialize")]
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
        #[serde(deserialize_with = "crate::identity::deserialize")]
        id: Uuid,
        /// 题目身份；全流程恰好覆盖文档题目一次。
        #[serde(deserialize_with = "crate::identity::deserialize_vec")]
        question_ids: Vec<Uuid>,
        /// 下一节点。
        #[serde(deserialize_with = "crate::identity::deserialize")]
        next: Uuid,
    },
    /// 自动条件分支，必须有默认出口。
    Branch {
        /// 分支节点身份。
        #[serde(deserialize_with = "crate::identity::deserialize")]
        id: Uuid,
        /// 有优先级的条件列表。
        branches: Vec<FlowBranch>,
        /// 所有条件均为 false 时的出口。
        #[serde(deserialize_with = "crate::identity::deserialize")]
        otherwise: Uuid,
    },
    /// 终点。
    End {
        /// 终点身份。
        #[serde(deserialize_with = "crate::identity::deserialize")]
        id: Uuid,
        /// 结果种类。
        outcome: FlowOutcome,
    },
}

impl FlowNode {
    pub(crate) fn id(&self) -> Uuid {
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

    pub fn renew_ids(
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

/// 编译后的流程；节点列表由 `Arc` 共享，克隆与求值器复用都只增加引用计数。
#[derive(Clone, Debug)]
pub(crate) struct CompiledFlow {
    nodes: Arc<[Node]>,
}

impl CompiledFlow {
    /// 未配置流程时的隐式单页流程。
    pub(crate) fn implicit(questions: &[Question]) -> Self {
        // 虚拟身份只存在于运行状态，不写入未配置 `logic` 的文档，也不改变版本指纹。
        Self {
            nodes: vec![
                Node::Page {
                    id: IMPLICIT_PAGE_ID,
                    questions: questions.iter().map(|question| question.id).collect(),
                    next: 1,
                },
                Node::End {
                    id: IMPLICIT_END_ID,
                    outcome: FlowOutcome::Complete,
                },
            ]
            .into(),
        }
    }
    fn id(&self, index: usize) -> Uuid {
        match &self.nodes[index] {
            Node::Page { id, .. } | Node::End { id, .. } => *id,
            // 编译器不会把分支节点写入访问路径，路径身份只来自页面与结束节点。
            Node::Branch { .. } => unreachable!("分支节点不进入访问路径"),
        }
    }
}

pub(crate) fn compile_flow(
    flow: &FlowDefinition,
    mut occupied: BTreeSet<Uuid>,
    logic: &CompiledDocumentLogic,
    questions: &[Question],
    context: &CompilationContext,
) -> Result<CompiledFlow, LogicError> {
    let invalid = || error(LogicErrorCode::InvalidFlow, "$.logic.flow", None);
    if flow.nodes.len() > MAX_FLOW_NODES {
        return Err(error(LogicErrorCode::LimitExceeded, "$.logic.flow", None));
    }
    if !matches!(flow.nodes.first(), Some(FlowNode::Page { id, .. }) if *id == flow.entry) {
        return Err(invalid());
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
            && (branches.is_empty() || branches.len() > MAX_BRANCHES)
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
    // 变量按拓扑顺序展开：每个变量只依赖更早的变量，因此读取集合可逐层复用。
    let mut variable_reads = BTreeMap::new();
    for id in logic.variable_order() {
        variable_reads.insert(
            *id,
            logic
                .variable_expression(*id)
                .required_reads(&variable_reads),
        );
    }
    let check_available =
        |expression: &CompiledExpression, node: usize, path: &str| -> Result<(), LogicError> {
            if expression
                .required_reads(&variable_reads)
                .iter()
                .any(|id| !dominators[node].contains(&pages[id]))
            {
                return Err(error(LogicErrorCode::UnavailableDependency, path, None));
            }
            Ok(())
        };
    for rule in logic.rules() {
        for (field, expression) in [
            ("visible_if", &rule.visible_if),
            ("required_if", &rule.required_if),
            ("validate", &rule.validate),
        ] {
            if let Some(expression) = expression {
                check_available(
                    expression,
                    pages[&rule.question_id],
                    &format!("{}.{field}", rule.path),
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
                    let expression = context
                        .compile(&branch.when)
                        .map_err(|failure| failure.relocate(&path))?;
                    if expression.value_type() != LogicValueType::Boolean {
                        return Err(error(LogicErrorCode::TypeMismatch, &path, None));
                    }
                    // 常量条件只能是不遮蔽后续分支的最后一条 true 分支。
                    if let Some(literal) = expression.literal_boolean()
                        && (!literal || branch_index + 1 < branches.len())
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
                    check_available(&expression, index, &path)?;
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
    Ok(CompiledFlow {
        nodes: nodes.into(),
    })
}

/// 题目身份到文档下标的一次性索引；流程编译保证页面题目都来自本文档。
pub(crate) fn index_questions(questions: &[Question]) -> BTreeMap<Uuid, usize> {
    questions
        .iter()
        .enumerate()
        .map(|(index, question)| (question.id, index))
        .collect()
}

/// 一个外部事件完整处理后才更新状态；答案使用与提交相同的 JSON 类型。
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum FlowEvent {
    /// 修改当前页面上的可见题目。
    Change {
        /// 题目身份。
        #[serde(deserialize_with = "crate::identity::deserialize")]
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
    required: Vec<Uuid>,
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
    /// 当前可见且必填的题目，包含动态必填。
    pub required: Vec<Uuid>,
    /// 已校验的有效答案。
    pub answers: BTreeMap<Uuid, Value>,
    /// 可见题目的错误；隐藏题目不留错误。
    pub errors: BTreeMap<Uuid, LogicErrorCode>,
    /// 路径末端的未决分支或计算错误。
    pub route_error: Option<LogicErrorCode>,
    /// 最近一次改答清除的题目身份。
    pub cleared: Vec<Uuid>,
}

/// 借用会话状态的只读快照；序列化字段与 [`FlowSnapshot`] 完全一致。
///
/// 大字段直接借用会话内部集合，避免每次按键或翻页复制全部答案与错误；
/// 只有历史与路径需要按下标映射为节点身份。
#[derive(Debug, Serialize)]
pub struct FlowSnapshotRef<'a> {
    /// 当前页面或结束节点。
    pub current_node: Uuid,
    /// 只有真正前进到结束节点后才有结果。
    pub outcome: Option<FlowOutcome>,
    /// 实际访问且仍有效的历史。
    pub history: Vec<Uuid>,
    /// 根据当前答案推导出的页面及结束节点路径。
    pub path: Vec<Uuid>,
    /// 当前路径上的可见题目。
    pub visible: &'a [Uuid],
    /// 当前可见且必填的题目，包含动态必填。
    pub required: &'a [Uuid],
    /// 已校验的有效答案。
    pub answers: &'a BTreeMap<Uuid, Value>,
    /// 可见题目的错误；隐藏题目不留错误。
    pub errors: &'a BTreeMap<Uuid, LogicErrorCode>,
    /// 路径末端的未决分支或计算错误。
    pub route_error: Option<LogicErrorCode>,
    /// 最近一次改答清除的题目身份。
    pub cleared: &'a [Uuid],
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

/// 已存储答案的重建结果：规范化答案与题目适用性（跳过路径分配）。
pub type StoredStates = (BTreeMap<Uuid, Value>, BTreeMap<Uuid, AnswerState>);

/// 只持有编译结果与不可变题面；交互会话和服务端重建共用一次顺序求值。
///
/// 编译产物通过 `Cow` 同时支持拥有（独立求值器）与借用（[`crate::domain::PreparedDocument`]
/// 一次性编译后复用），构造求值器不再复制规则、流程或题目表。
#[derive(Debug)]
pub struct FlowEvaluator<'a> {
    document: &'a Document,
    questions: Cow<'a, BTreeMap<Uuid, usize>>,
    logic: Cow<'a, CompiledDocumentLogic>,
    flow: Cow<'a, CompiledFlow>,
}

impl<'a> FlowEvaluator<'a> {
    /// 编译并绑定文档；同一发布版本可复用此实例重建多份答卷。
    ///
    /// # 错误
    /// 草稿结构、规则或流程不合法时返回定位错误。
    pub fn new(document: &'a Document) -> Result<Self, LogicError> {
        document
            .validate_draft_fields()
            .map_err(|failure| error(LogicErrorCode::InvalidFlow, "$", failure.question_id))?;
        let logic = match &document.logic {
            Some(logic) => logic.compile(&document.questions)?,
            None => CompiledDocumentLogic::empty(),
        };
        Ok(Self::from_compiled(document, logic))
    }

    /// 调用方保证文档已通过草稿字段校验（发布版本在发布边界已完成校验），并传入该文档的编译结果。
    pub(crate) fn from_compiled(document: &'a Document, logic: CompiledDocumentLogic) -> Self {
        let flow = logic.effective_flow(&document.questions);
        Self {
            document,
            questions: Cow::Owned(index_questions(&document.questions)),
            logic: Cow::Owned(logic),
            flow: Cow::Owned(flow),
        }
    }

    /// 零拷贝借用已编译的文档产物；仅由 [`crate::domain::PreparedDocument`] 使用。
    pub(crate) fn from_prepared(
        document: &'a Document,
        questions: &'a BTreeMap<Uuid, usize>,
        logic: &'a CompiledDocumentLogic,
        flow: &'a CompiledFlow,
    ) -> Self {
        Self {
            document,
            questions: Cow::Borrowed(questions),
            logic: Cow::Borrowed(logic),
            flow: Cow::Borrowed(flow),
        }
    }

    /// 从入口重建最终答卷，不接受客户端历史或预先计算的可见题目。
    ///
    /// # 错误
    /// 重复/未知题目、不适用答案、校验失败、未决条件或筛选结束均拒绝。
    pub fn evaluate_answers(&self, answers: Vec<Answer>) -> Result<EvaluatedResponse, LogicError> {
        let inputs = self.collect_inputs(answers)?;
        self.finish(&inputs)
    }

    /// 从数据库中的已规范化答案重算规则、范围和路径。
    pub fn evaluate_stored(
        &self,
        answers: &BTreeMap<Uuid, Value>,
    ) -> Result<EvaluatedResponse, LogicError> {
        self.finish(answers)
    }

    /// 校验答案并返回规范化结果，跳过 `question_states` 与 `path` 的构建分配。
    ///
    /// # 错误
    /// 与 [`Self::evaluate_answers`] 相同。
    pub fn validate_answers_only(
        &self,
        answers: Vec<Answer>,
    ) -> Result<BTreeMap<Uuid, Value>, LogicError> {
        let inputs = self.collect_inputs(answers)?;
        let projection = self.project(&inputs);
        self.check_projection(&projection, &inputs)?;
        Ok(projection.answers)
    }

    /// 从已存储答案重算规则并返回规范化答案与题目适用性，跳过路径构建。
    ///
    /// # 错误
    /// 与 [`Self::evaluate_stored`] 相同。
    pub fn evaluate_stored_states(
        &self,
        answers: &BTreeMap<Uuid, Value>,
    ) -> Result<StoredStates, LogicError> {
        let projection = self.project(answers);
        let visible = self.check_projection(&projection, answers)?;
        let states = Self::question_states(&self.document.questions, &projection, &visible);
        Ok((projection.answers, states))
    }

    fn collect_inputs(&self, answers: Vec<Answer>) -> Result<BTreeMap<Uuid, Value>, LogicError> {
        let mut inputs = BTreeMap::new();
        for answer in answers {
            if !self.questions.contains_key(&answer.question_id) {
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
        Ok(inputs)
    }

    fn finish(&self, inputs: &BTreeMap<Uuid, Value>) -> Result<EvaluatedResponse, LogicError> {
        let projection = self.project(inputs);
        let visible = self.check_projection(&projection, inputs)?;
        let question_states =
            Self::question_states(&self.document.questions, &projection, &visible);
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

    /// 校验投影，错误顺序与提交路径完全一致；返回可见题目集合。
    fn check_projection(
        &self,
        projection: &Projection,
        inputs: &BTreeMap<Uuid, Value>,
    ) -> Result<BTreeSet<Uuid>, LogicError> {
        for question in &self.document.questions {
            if let Some(code) = projection.errors.get(&question.id) {
                return Err(error(*code, "$.answers", Some(question.id)));
            }
        }
        if let Some(code) = projection.route_error {
            return Err(error(code, "$.route", None));
        }
        let visible: BTreeSet<Uuid> = projection.visible.iter().copied().collect();
        for id in inputs.keys() {
            if !visible.contains(id) {
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
        Ok(visible)
    }

    fn question_states(
        questions: &[Question],
        projection: &Projection,
        visible: &BTreeSet<Uuid>,
    ) -> BTreeMap<Uuid, AnswerState> {
        questions
            .iter()
            .map(|question| {
                let state = if !visible.contains(&question.id) {
                    AnswerState::Skipped
                } else if projection.answers.contains_key(&question.id) {
                    AnswerState::Answered
                } else {
                    AnswerState::Unanswered
                };
                (question.id, state)
            })
            .collect()
    }

    fn project(&self, inputs: &BTreeMap<Uuid, Value>) -> Projection {
        // 路径最多覆盖全部流程节点，可见与必填题目最多覆盖全部题目；一次预分配避免扩容。
        let mut result = Projection {
            path: Vec::with_capacity(self.flow.nodes.len()),
            visible: Vec::with_capacity(self.document.questions.len()),
            required: Vec::with_capacity(self.document.questions.len()),
            ..Projection::default()
        };
        // 前置依赖规则保证：被当前表达式引用的变量所依赖的题目均已在前面求值定型，
        // 因此同一投影过程可安全复用计算变量缓存。
        let mut variables = BTreeMap::new();
        let mut current = 0;
        loop {
            match &self.flow.nodes[current] {
                Node::Page {
                    questions, next, ..
                } => {
                    result.path.push(current);
                    for id in questions {
                        // 流程编译保证页面题目都来自本文档，索引不会缺失。
                        let question = &self.document.questions[self.questions[id]];
                        let rule = self.logic.rule(*id);
                        match rule
                            .and_then(|rule| rule.visible_if.as_ref())
                            .map(|expression| {
                                self.logic.evaluate_expression(
                                    expression,
                                    &result.answers,
                                    &mut variables,
                                )
                            })
                            .unwrap_or(Ok(LogicValue::Boolean(true)))
                        {
                            Ok(LogicValue::Boolean(true)) => {}
                            Ok(_) => continue,
                            Err(failure) => {
                                result.visible.push(*id);
                                result.errors.insert(*id, failure.code);
                                continue;
                            }
                        }
                        result.visible.push(*id);
                        let required = if question.required {
                            Ok(LogicValue::Boolean(true))
                        } else {
                            rule.and_then(|rule| rule.required_if.as_ref())
                                .map(|expression| {
                                    self.logic.evaluate_expression(
                                        expression,
                                        &result.answers,
                                        &mut variables,
                                    )
                                })
                                .unwrap_or(Ok(LogicValue::Boolean(false)))
                        };
                        if matches!(&required, Ok(LogicValue::Boolean(true))) {
                            result.required.push(*id);
                        }
                        let value = match question.normalize_answer(inputs.get(id)) {
                            Ok(value) => value,
                            Err(_) => {
                                result.errors.insert(*id, LogicErrorCode::InvalidAnswer);
                                continue;
                            }
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
                            if let Some(validate) = rule.and_then(|rule| rule.validate.as_ref()) {
                                match self.logic.evaluate_expression(
                                    validate,
                                    &result.answers,
                                    &mut variables,
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
                            }
                            if result.errors.contains_key(id) {
                                result.answers.remove(id);
                                self.logic.evict_dependent_variables(&mut variables, *id);
                            }
                        }
                    }
                    current = *next;
                }
                Node::Branch { guards, otherwise } => {
                    let mut target = *otherwise;
                    for (guard, next) in guards {
                        match self
                            .logic
                            .evaluate_expression(guard, &result.answers, &mut variables)
                        {
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
    /// 绑定不可变文档并创建空填写会话；未配置 `logic` 的问卷使用内存中的单页流程。
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

    /// 复用已绑定的求值器重建最终答卷，不读取或修改会话的输入与历史。
    ///
    /// # 错误
    /// 与独立求值器相同，拒绝无效答案、不适用答案、未决条件或筛选结束。
    pub fn evaluate_answers(&self, answers: Vec<Answer>) -> Result<EvaluatedResponse, LogicError> {
        self.evaluator.evaluate_answers(answers)
    }

    /// 返回借用会话状态的只读快照，避免复制全部答案与可见题目。
    #[must_use]
    pub fn snapshot_ref(&self) -> FlowSnapshotRef<'_> {
        let current = self.current();
        FlowSnapshotRef {
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
            visible: &self.projection.visible,
            required: &self.projection.required,
            answers: &self.projection.answers,
            errors: &self.projection.errors,
            route_error: self.projection.route_error,
            cleared: &self.cleared,
        }
    }

    /// 返回独立的只读快照，外部修改不会改变会话。
    #[must_use]
    pub fn snapshot(&self) -> FlowSnapshot {
        let snapshot = self.snapshot_ref();
        FlowSnapshot {
            current_node: snapshot.current_node,
            outcome: snapshot.outcome,
            history: snapshot.history,
            path: snapshot.path,
            visible: snapshot.visible.to_vec(),
            required: snapshot.required.to_vec(),
            answers: snapshot.answers.clone(),
            errors: snapshot.errors.clone(),
            route_error: snapshot.route_error,
            cleared: snapshot.cleared.to_vec(),
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
                // 拒绝检查已结束；投影没有可恢复失败，不需要复制输入表来回滚。
                self.inputs.insert(question_id, value);
                let projection = self.evaluator.project(&self.inputs);
                // 可见集合与路径集合各扫一次，替代对每个输入和历史的线性查找；
                // 单遍裁剪输入时同步收集被清除的题目。
                let visible: BTreeSet<Uuid> = projection.visible.iter().copied().collect();
                let path: BTreeSet<usize> = projection.path.iter().copied().collect();
                let cleared = &mut self.cleared;
                cleared.clear();
                self.inputs.retain(|id, _| {
                    if visible.contains(id) {
                        true
                    } else {
                        cleared.push(*id);
                        false
                    }
                });
                self.history.retain(|index| path.contains(index));
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

#[cfg(test)]
mod tests {
    use serde_json::{Number, json};
    use uuid::Uuid;

    use super::*;
    use crate::domain::{Choice, Document, DocumentLogic, Question, QuestionKind, QuestionRule};

    fn id(value: u128) -> Uuid {
        Uuid::from_u128(value)
    }

    fn 版本() -> Number {
        Number::from(1_u32)
    }

    fn 选项(value: u128) -> Choice {
        Choice {
            id: id(value),
            label: format!("选项 {value}"),
        }
    }

    fn 单选题(value: u128, options: &[u128]) -> Question {
        Question {
            id: id(value),
            title: format!("题目 {value}"),
            required: false,
            kind: QuestionKind::SingleChoice {
                options: options.iter().map(|option| 选项(*option)).collect(),
            },
        }
    }

    fn 文档(questions: Vec<Question>, logic: Option<DocumentLogic>) -> Document {
        Document {
            logic,
            title: "流程问卷".into(),
            description: String::new(),
            purpose: "收集反馈".into(),
            thank_you: String::new(),
            questions,
        }
    }

    fn 逻辑(flow: FlowDefinition) -> DocumentLogic {
        DocumentLogic {
            flow: Some(flow),
            dsl_version: 版本(),
            semantics_version: 版本(),
            variables: Vec::new(),
            rules: Vec::new(),
        }
    }

    fn 逻辑带规则(flow: FlowDefinition, rules: Vec<QuestionRule>) -> DocumentLogic {
        DocumentLogic {
            flow: Some(flow),
            dsl_version: 版本(),
            semantics_version: 版本(),
            variables: Vec::new(),
            rules,
        }
    }

    fn 页面(value: u128, questions: &[u128], next: u128) -> FlowNode {
        FlowNode::Page {
            id: id(value),
            question_ids: questions.iter().map(|question| id(*question)).collect(),
            next: id(next),
        }
    }

    fn 结束(value: u128) -> FlowNode {
        FlowNode::End {
            id: id(value),
            outcome: FlowOutcome::Complete,
        }
    }

    fn 分支流程(branches: Vec<FlowBranch>) -> FlowDefinition {
        FlowDefinition {
            entry: id(201),
            nodes: vec![
                页面(201, &[1], 202),
                FlowNode::Branch {
                    id: id(202),
                    branches,
                    otherwise: id(203),
                },
                结束(203),
            ],
        }
    }

    fn 编译流程(questions: &[Question], logic: &DocumentLogic) -> CompiledFlow {
        logic
            .compile(questions)
            .unwrap_or_else(|error| panic!("流程应可编译：{error:?}"))
            .flow
            .expect("配置了流程时必须产出编译结果")
    }

    #[test]
    fn 流程结构拒绝空节点非首页与空页面() {
        let questions = [单选题(1, &[101])];

        let 空节点 = FlowDefinition {
            entry: id(201),
            nodes: Vec::new(),
        };
        let 错误 = 逻辑(空节点).compile(&questions).unwrap_err();
        assert_eq!(错误.code, LogicErrorCode::InvalidFlow);
        assert_eq!(错误.path, "$.logic.flow");

        // 入口必须指向首个页面节点。
        let 入口错误 = FlowDefinition {
            entry: id(202),
            nodes: vec![页面(201, &[1], 202), 结束(202)],
        };
        let 错误 = 逻辑(入口错误).compile(&questions).unwrap_err();
        assert_eq!(错误.code, LogicErrorCode::InvalidFlow);
        assert_eq!(错误.path, "$.logic.flow");
        assert_eq!(错误.question_id, None);

        // 页面不能没有题目。
        let 空页面 = FlowDefinition {
            entry: id(201),
            nodes: vec![页面(201, &[], 202), 结束(202)],
        };
        let 错误 = 逻辑(空页面).compile(&questions).unwrap_err();
        assert_eq!(错误.code, LogicErrorCode::InvalidFlow);
        assert_eq!(错误.path, "$.logic.flow");
        assert_eq!(错误.question_id, None);
    }

    #[test]
    fn 分支数量与常量条件遵守边界() {
        let questions = [单选题(1, &[101])];
        let 守 = || json!({"op": "answered", "question_id": id(1).to_string()});
        let 真 = || json!({"op": "boolean", "value": true});

        // 空分支列表无法给出默认出口。
        let 错误 = 逻辑(分支流程(Vec::new())).compile(&questions).unwrap_err();
        assert_eq!(错误.code, LogicErrorCode::InvalidFlow);
        assert_eq!(错误.path, "$.logic.flow");

        // 恰好 MAX_BRANCHES 个条件可以编译，且常量 true 允许出现在末位。
        let mut 满分 = (0..MAX_BRANCHES - 1)
            .map(|_| FlowBranch {
                when: 守(),
                target: id(203),
            })
            .collect::<Vec<_>>();
        满分.push(FlowBranch {
            when: 真(),
            target: id(203),
        });
        let flow = 编译流程(&questions, &逻辑(分支流程(满分)));
        match &flow.nodes[1] {
            Node::Branch { guards, otherwise } => {
                assert_eq!(guards.len(), MAX_BRANCHES);
                assert_eq!(*otherwise, 2, "默认出口指向结束节点索引");
            }
            _ => panic!("第二节点必须是分支"),
        }

        // 超过 MAX_BRANCHES 直接拒绝。
        let mut 超限 = (0..MAX_BRANCHES)
            .map(|_| FlowBranch {
                when: 守(),
                target: id(203),
            })
            .collect::<Vec<_>>();
        超限.push(FlowBranch {
            when: 真(),
            target: id(203),
        });
        let 错误 = 逻辑(分支流程(超限)).compile(&questions).unwrap_err();
        assert_eq!(错误.code, LogicErrorCode::InvalidFlow);

        // 常量 true 位于末位仅作为默认判定，允许编译。
        assert!(
            逻辑(分支流程(vec![
                FlowBranch {
                    when: 守(),
                    target: id(203),
                },
                FlowBranch {
                    when: 真(),
                    target: id(203),
                },
            ]))
            .compile(&questions)
            .is_ok()
        );
    }

    #[test]
    fn 改答只接受当前页面的可见题目() {
        // 当前页被显示规则隐藏的题目同样不能改答。
        let 隐藏问卷 = 文档(
            vec![单选题(1, &[101, 102]), 单选题(2, &[103, 104])],
            Some(逻辑带规则(
                FlowDefinition {
                    entry: id(201),
                    nodes: vec![页面(201, &[1, 2], 202), 结束(202)],
                },
                vec![QuestionRule {
                    question_id: id(2),
                    visible_if: Some(json!({"op": "boolean", "value": false})),
                    required_if: None,
                    validate: None,
                }],
            )),
        );
        let mut 会话 = FlowSession::new(&隐藏问卷).unwrap();
        assert_eq!(会话.snapshot().visible, vec![id(1)]);
        let 错误 = 会话
            .dispatch(FlowEvent::Change {
                question_id: id(2),
                value: json!(id(103).to_string()),
            })
            .unwrap_err();
        assert_eq!(错误.code, LogicErrorCode::InvalidEvent, "不能改答隐藏题目");
        assert_eq!(错误.path, "$.event");
    }
}
