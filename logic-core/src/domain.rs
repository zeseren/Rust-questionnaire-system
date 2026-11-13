//! 问卷内容与答案的领域规则，不依赖 HTTP、数据库或当前登录用户。
//!
//! 草稿可以暂时没有标题、题目或完整配置，但始终受容量和身份唯一性限制。
//! 所有文字长度均按 Unicode 标量值（`chars()`）计数，而不是 UTF-8 字节数。
//! 所有题面和文本答案都拒绝 U+0000 空字符，避免文本无法持久化。

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::flow::CompiledPlan;
pub use crate::flow::{
    AnswerState, EvaluatedResponse, FlowBranch, FlowDefinition, FlowEvaluator, FlowEvent, FlowNode,
    FlowOutcome, FlowSession, FlowSnapshot, StoredStates,
};
pub use crate::logic::{
    CompiledExpression, LogicError, LogicErrorCode, LogicValue, LogicValueType, compile_expression,
};
pub use crate::logic_document::{
    CompiledDocumentLogic, ComputedVariable, DocumentLogic, DocumentLogicValues, QuestionRule,
    RuleValues,
};
pub use crate::logic_text::{
    LogicTextBinding, LogicTextError, LogicTextInspection, format_logic_text, inspect_logic_text,
    parse_logic_text,
};

/// 每份问卷最多允许的题目数。
pub const MAX_QUESTIONS: usize = 100;
/// 每道选择题最多允许的选项数。
pub const MAX_CHOICES: usize = 50;
/// 调查说明与收集目的最多允许的 Unicode 标量值数。
pub const MAX_TEXT_LENGTH: usize = 5000;
/// 草稿与发布版本的单行文本答案最多允许的 Unicode 标量值数。
pub const MAX_TEXT_ANSWER_LENGTH: usize = 200;
/// 草稿与发布版本的多行文本答案最多允许的 Unicode 标量值数。
pub const MAX_TEXTAREA_ANSWER_LENGTH: usize = 1000;
/// 单次提交请求体的字节上限（HTTP body limit）。
pub const MAX_SUBMISSION_BYTES: usize = 1024 * 1024;

/// 问卷标题的 Unicode 标量值上限。
const MAX_DOCUMENT_TITLE_LENGTH: usize = 200;
/// 单道题目标题的 Unicode 标量值上限。
const MAX_QUESTION_TITLE_LENGTH: usize = 500;
/// 单个选项显示文字的 Unicode 标量值上限。
const MAX_CHOICE_LABEL_LENGTH: usize = 500;
/// 提交结束语的 Unicode 标量值上限。
const MAX_THANK_YOU_LENGTH: usize = 1000;
/// 评分端点说明的 Unicode 标量值上限。
const MAX_RATING_LABEL_LENGTH: usize = 100;

/// 草稿或发布版本中的完整问卷内容；数组顺序就是题目显示顺序。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Document {
    /// 可选的版本化规则；未配置 `logic` 的文档序列化时不增加字段。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logic: Option<DocumentLogic>,
    /// 问卷标题，最多 200 个 Unicode 标量值；发布时不能为空白。
    pub title: String,
    /// 调查说明，最多 5000 个 Unicode 标量值。
    pub description: String,
    /// 收集目的，最多 5000 个 Unicode 标量值；发布时不能为空白。
    pub purpose: String,
    /// 提交后的结束语，最多 1000 个 Unicode 标量值。
    pub thank_you: String,
    /// 按显示顺序排列的题目，最多 100 题。
    pub questions: Vec<Question>,
}

/// 一道题目；修改内容或排序不能更改其 ID，复制时才生成新 ID。
#[derive(Clone, Debug, PartialEq, Serialize, ToSchema)]
pub struct Question {
    /// 在问卷内唯一且非空的稳定题目 ID。
    pub id: Uuid,
    /// 题目标题，最多 500 个 Unicode 标量值；发布时不能为空白。
    pub title: String,
    /// 是否必须作答；空答案不会自动转换成零分或默认选项。
    pub required: bool,
    /// 题型配置在 JSON 中与 `id`、`title`、`required` 位于同一层。
    #[serde(flatten)]
    pub kind: QuestionKind,
}

impl<'de> Deserialize<'de> for Question {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // 公共字段按目标类型直接读取，剩余字段原样交给严格的题型枚举。
        // 不用 `Value::Object`：那会把每道题的选项数组先整体物化成 `serde_json::Value`，
        // 再二次反序列化为强类型，产生一次多余的全量堆分配。
        #[derive(Deserialize)]
        struct Wire {
            #[serde(deserialize_with = "crate::identity::deserialize")]
            id: Uuid,
            title: String,
            required: bool,
            // 题型枚举自带 `deny_unknown_fields`，拼错的配置项仍会被拒绝而不是丢弃；
            // 这里不能用 `deny_unknown_fields`，Serde 不允许它与 `flatten` 同时出现。
            #[serde(flatten)]
            kind: QuestionKind,
        }

        let wire = Wire::deserialize(deserializer)?;
        Ok(Self {
            id: wire.id,
            title: wire.title,
            required: wire.required,
            kind: wire.kind,
        })
    }
}

/// 六种题型的配置；JSON 使用 `type` 字段和 `snake_case` 题型名区分。
/// 除 `type` 外只接受所选题型自己的配置项，未知字段直接拒绝。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum QuestionKind {
    /// 单选题，答案是一个合法选项的 UUID 字符串。
    SingleChoice {
        /// 按显示顺序排列的选项；发布时至少 2 个，最多 50 个。
        options: Vec<Choice>,
    },
    /// 多选题，答案是无重复的选项 UUID 数组，排列顺序不改变含义。
    MultipleChoice {
        /// 按显示顺序排列的选项；发布时至少 2 个，最多 50 个。
        options: Vec<Choice>,
        /// 作答时最少选择数；可选题完全不回答时不受此下限限制。
        min_choices: usize,
        /// 作答时最多选择数；发布时必须介于 1 和选项总数之间。
        max_choices: usize,
    },
    /// 单行文本题，不允许 Unicode 换行字符。
    Text {
        /// 最多允许的 Unicode 标量值数；须介于 1 和 200 之间。
        max_length: usize,
    },
    /// 多行文本题，允许换行。
    Textarea {
        /// 最多允许的 Unicode 标量值数；须介于 1 和 1000 之间。
        max_length: usize,
    },
    /// 数字题，只接受有限的 JSON 数字，不接受字符串、布尔值或非有限数值。
    Number {
        /// 为 true 时只接受数学意义上的整数，`1` 与 `1.0` 等价。
        integer: bool,
        /// 可选的闭区间下界，必须是有限数值。
        min: Option<f64>,
        /// 可选的闭区间上界，必须是有限数值。
        max: Option<f64>,
    },
    /// 评分题，答案必须是指定闭区间内的整数。
    Rating {
        /// 最低分，允许负数和零。
        min: i32,
        /// 最高分，发布时不得小于最低分。
        max: i32,
        /// 最低分说明，最多 100 个 Unicode 标量值，可以留空。
        min_label: String,
        /// 最高分说明，最多 100 个 Unicode 标量值，可以留空。
        max_label: String,
    },
}

impl QuestionKind {
    /// 选择题的选项；非选择题返回 `None`。
    #[must_use]
    pub fn options(&self) -> Option<&[Choice]> {
        match self {
            Self::SingleChoice { options } | Self::MultipleChoice { options, .. } => Some(options),
            Self::Text { .. }
            | Self::Textarea { .. }
            | Self::Number { .. }
            | Self::Rating { .. } => None,
        }
    }

    /// 选择题的可变选项；非选择题返回 `None`。
    pub fn options_mut(&mut self) -> Option<&mut Vec<Choice>> {
        match self {
            Self::SingleChoice { options } | Self::MultipleChoice { options, .. } => Some(options),
            Self::Text { .. }
            | Self::Textarea { .. }
            | Self::Number { .. }
            | Self::Rating { .. } => None,
        }
    }
}

/// 选择题的一个选项；文字可以重复，身份 ID 不能重复。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    /// 在整份问卷中唯一且非空的稳定选项 ID。
    #[serde(deserialize_with = "crate::identity::deserialize")]
    pub id: Uuid,
    /// 显示文字，最多 500 个 Unicode 标量值；发布时不能为空白。
    pub label: String,
}

/// 提交的单题答案；使用数组接收，避免 JSON 对象提前覆盖重复题目。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    /// 此答案所属的稳定题目 ID。
    #[serde(deserialize_with = "crate::identity::deserialize")]
    pub question_id: Uuid,
    /// 随题型而定的 JSON 值；`null` 表示未回答。
    pub value: Value,
}

/// 可定位到题目的校验错误；问卷整体错误的 `question_id` 为 `None`。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, thiserror::Error)]
#[error("{message}")]
pub struct ValidationError {
    /// 错误涉及的题目 ID，保留未知或重复提交的 ID 以便前端定位。
    pub question_id: Option<Uuid>,
    /// 可以展示给用户的中文错误说明，不包含答案正文。
    pub message: String,
}

impl ValidationError {
    fn new(question_id: Option<Uuid>, message: impl Into<String>) -> Self {
        Self {
            question_id,
            message: message.into(),
        }
    }
}

impl Document {
    /// 校验草稿的容量、字段长度、稳定 ID 和数值有限性。
    ///
    /// 允许空问卷、空白标题、暂缺选项及暂时矛盾的配置范围；规则与流程可以
    /// 处于未写完的中间状态，发布或提交时才要求编译通过。
    /// 此函数不修改内容，也不会自动生成或修复重复 ID。
    ///
    /// # Errors
    /// 超过容量、文本含空字符、出现空或重复 ID、数值边界非有限时返回可定位的错误。
    pub fn validate_draft(&self) -> Result<(), ValidationError> {
        self.validate_draft_fields()
    }

    /// 草稿字段校验通过后编译规则，供发布校验与发布态文档使用。
    fn validate_compiled(&self) -> Result<Option<CompiledDocumentLogic>, ValidationError> {
        self.validate_draft_fields()?;
        self.compile_logic()
            .map_err(|error| ValidationError::new(error.question_id, error.to_string()))
    }

    pub(crate) fn validate_draft_fields(&self) -> Result<(), ValidationError> {
        check_length(&self.title, MAX_DOCUMENT_TITLE_LENGTH, None, "问卷标题")?;
        check_length(&self.description, MAX_TEXT_LENGTH, None, "调查说明")?;
        check_length(&self.purpose, MAX_TEXT_LENGTH, None, "收集目的")?;
        check_length(&self.thank_you, MAX_THANK_YOU_LENGTH, None, "结束语")?;
        if self.questions.len() > MAX_QUESTIONS {
            return Err(ValidationError::new(
                None,
                format!("每份问卷最多允许 {MAX_QUESTIONS} 道题目"),
            ));
        }

        // 选项总数可一次性算出，避免校验期间为集合反复扩容。
        let option_capacity = self
            .questions
            .iter()
            .map(|question| question.kind.options().map_or(0, <[Choice]>::len))
            .sum();
        let mut question_ids = HashSet::with_capacity(self.questions.len());
        let mut option_ids = HashSet::with_capacity(option_capacity);
        for question in &self.questions {
            let id = Some(question.id);
            if question.id.is_nil() || !question_ids.insert(question.id) {
                return Err(ValidationError::new(id, "题目 ID 不能为空或重复"));
            }
            check_length(&question.title, MAX_QUESTION_TITLE_LENGTH, id, "题目标题")?;
            if let Some(options) = question.kind.options() {
                if options.len() > MAX_CHOICES {
                    return Err(ValidationError::new(
                        id,
                        format!("每道选择题最多允许 {MAX_CHOICES} 个选项"),
                    ));
                }
                for option in options {
                    if option.id.is_nil() || !option_ids.insert(option.id) {
                        return Err(ValidationError::new(id, "选项 ID 不能为空或在问卷中重复"));
                    }
                    check_length(&option.label, MAX_CHOICE_LABEL_LENGTH, id, "选项文字")?;
                }
            }
            match &question.kind {
                QuestionKind::MultipleChoice {
                    min_choices,
                    max_choices,
                    ..
                } => {
                    if *min_choices > MAX_CHOICES || *max_choices > MAX_CHOICES {
                        return Err(ValidationError::new(
                            id,
                            format!("选择数量配置不能超过 {MAX_CHOICES}"),
                        ));
                    }
                }
                QuestionKind::Text { max_length } => {
                    if *max_length > MAX_TEXT_ANSWER_LENGTH {
                        return Err(ValidationError::new(
                            id,
                            format!("单行文本长度配置不能超过 {MAX_TEXT_ANSWER_LENGTH}"),
                        ));
                    }
                }
                QuestionKind::Textarea { max_length } => {
                    if *max_length > MAX_TEXTAREA_ANSWER_LENGTH {
                        return Err(ValidationError::new(
                            id,
                            format!("多行文本长度配置不能超过 {MAX_TEXTAREA_ANSWER_LENGTH}"),
                        ));
                    }
                }
                QuestionKind::Number { min, max, .. } => {
                    if min.is_some_and(|value| !value.is_finite())
                        || max.is_some_and(|value| !value.is_finite())
                    {
                        return Err(ValidationError::new(id, "数值上下界必须是有限数值"));
                    }
                }
                QuestionKind::Rating {
                    min_label,
                    max_label,
                    ..
                } => {
                    check_length(min_label, MAX_RATING_LABEL_LENGTH, id, "最低分说明")?;
                    check_length(max_label, MAX_RATING_LABEL_LENGTH, id, "最高分说明")?;
                }
                QuestionKind::SingleChoice { .. } => {}
            }
        }
        Ok(())
    }

    /// 编译文档规则；未配置 `logic` 的文档返回 None。
    ///
    /// # Errors
    /// 规则类型、引用、容量或依赖关系不合法时返回定位信息。
    pub fn compile_logic(&self) -> Result<Option<CompiledDocumentLogic>, LogicError> {
        self.logic
            .as_ref()
            .map(|logic| logic.compile(&self.questions))
            .transpose()
    }

    /// 在草稿基本校验之上检查问卷是否已完整到可以发布。
    ///
    /// 要求标题、收集目的、题目和选项文字完整，并检查六种题型的配置。
    /// 选项文字和题目标题可以相同，它们通过稳定 ID 区分。
    ///
    /// # Errors
    /// 草稿不合法、内容未补全、范围矛盾或题型没有合法答案时返回错误。
    pub fn validate_publish(&self) -> Result<(), ValidationError> {
        self.validate_publish_compiled().map(|_| ())
    }

    fn validate_publish_compiled(&self) -> Result<Option<CompiledDocumentLogic>, ValidationError> {
        let logic = self.validate_compiled()?;
        if self.title.trim().is_empty() {
            return Err(ValidationError::new(None, "发布前请填写问卷标题"));
        }
        if self.purpose.trim().is_empty() {
            return Err(ValidationError::new(None, "发布前请说明数据收集目的"));
        }
        if self.questions.is_empty() {
            return Err(ValidationError::new(None, "发布前至少需要一道题目"));
        }

        for question in &self.questions {
            let id = Some(question.id);
            if question.title.trim().is_empty() {
                return Err(ValidationError::new(id, "发布前请填写题目标题"));
            }
            if let Some(options) = question.kind.options() {
                if options.len() < 2 {
                    return Err(ValidationError::new(id, "选择题至少需要两个选项"));
                }
                if options.iter().any(|option| option.label.trim().is_empty()) {
                    return Err(ValidationError::new(id, "发布前请填写所有选项文字"));
                }
                if let QuestionKind::MultipleChoice {
                    min_choices,
                    max_choices,
                    ..
                } = &question.kind
                    && (*max_choices == 0
                        || min_choices > max_choices
                        || *max_choices > options.len())
                {
                    return Err(ValidationError::new(
                        id,
                        "选择数量上下界必须有序，且最大值介于 1 和选项总数之间",
                    ));
                }
            }
            match &question.kind {
                QuestionKind::SingleChoice { .. } | QuestionKind::MultipleChoice { .. } => {}
                QuestionKind::Text { max_length } | QuestionKind::Textarea { max_length } => {
                    if *max_length == 0 {
                        return Err(ValidationError::new(id, "文本最大长度至少为 1"));
                    }
                }
                QuestionKind::Number { integer, min, max } => {
                    if let (Some(min), Some(max)) = (min, max)
                        && (min > max || (*integer && min.ceil() > max.floor()))
                    {
                        return Err(ValidationError::new(
                            id,
                            "数值上下界矛盾或区间内没有合法整数",
                        ));
                    }
                }
                QuestionKind::Rating { min, max, .. } => {
                    if min > max {
                        return Err(ValidationError::new(id, "最高分不能小于最低分"));
                    }
                }
            }
        }
        Ok(logic)
    }

    /// 为复制出的问卷重新生成所有题目和选项的 UUIDv7，并按映射重写规则引用。
    ///
    /// 除身份外，内容、规则和显示顺序均保持不变；普通保存和排序不要调用。
    pub fn renew_ids(&mut self) {
        let mut question_ids = BTreeMap::new();
        let mut option_ids = BTreeMap::new();
        for question in &mut self.questions {
            let new_id = Uuid::now_v7();
            question_ids.insert(question.id, new_id);
            question.id = new_id;
            if let Some(options) = question.kind.options_mut() {
                for option in options {
                    let new_id = Uuid::now_v7();
                    option_ids.insert(option.id, new_id);
                    option.id = new_id;
                }
            }
        }
        if let Some(logic) = &mut self.logic {
            logic.renew_references(&question_ids, &option_ids);
        }
    }

    /// 校验答案并生成可以稳定序列化、用于存储和幂等比较的规范化结果。
    ///
    /// 只校验答案与题目规则、流程路径的相容性；标题和说明等发布态文案已由
    /// 发布流程保证，不在每次提交时重复扫描。答案长度按版本内每题保存的
    /// `max_length` 检查。题目按 UUID 排序，多选按选项 UUID 排序，
    /// 数字统一为有限 `f64` JSON 数字，`1` 与 `1.0`、正零与负零分别等价。
    /// `null`、空白文本、空多选视为缺失；可选题不存储缺失项，必填题拒绝缺失。
    /// 非空白文本保留原文，不修剪、不截断。
    ///
    /// # Errors
    /// 文档结构不合法，或答案存在未知/重复题目、重复/非法选项、类型和范围错误、
    /// 超长文本、空字符、单行换行、缺失必填项时，返回包含相关题目 ID 的错误。
    pub fn validate_answers(
        &self,
        answers: Vec<Answer>,
    ) -> Result<BTreeMap<Uuid, Value>, ValidationError> {
        // 提交只复用规则的编译结果与同一段流程求值；发布态文案校验只发生在发布时。
        let logic = self
            .compile_logic()
            .map_err(|error| ValidationError::new(error.question_id, error.to_string()))?;
        CompiledPlan::from_compiled(self, logic)
            .validate_answers_only(self, answers)
            .map_err(|error| ValidationError::new(error.question_id, error.to_string()))
    }

    /// 执行完整发布校验并一次性编译规则、流程与题目索引。
    ///
    /// # Errors
    /// 草稿不合法、内容未补全、范围矛盾或题型没有合法答案时返回错误。
    pub fn prepare_publish(self) -> Result<PreparedDocument, ValidationError> {
        PreparedDocument::publish(self)
    }
}

/// 已完成草稿字段校验、规则编译、流程构建与题目索引的不可变发布态文档。
///
/// 编译计划只构建一次：提交校验与报告重建都复用同一份规则与流程，
/// 不再为每次请求重新编译 DSL。
#[derive(Debug)]
pub struct PreparedDocument {
    document: Arc<Document>,
    plan: CompiledPlan,
}

impl PreparedDocument {
    /// 校验草稿字段并一次性编译规则、流程与题目索引。
    ///
    /// # Errors
    /// 草稿字段、规则或流程不合法时返回可定位的校验错误。
    pub fn new(document: Document) -> Result<Self, ValidationError> {
        let logic = document.validate_compiled()?;
        Ok(Self::from_compiled(document, logic))
    }

    /// 执行完整发布校验并一次性编译规则、流程与题目索引。
    ///
    /// # Errors
    /// 草稿不合法、内容未补全、范围矛盾或题型没有合法答案时返回错误。
    pub fn publish(document: Document) -> Result<Self, ValidationError> {
        let logic = document.validate_publish_compiled()?;
        Ok(Self::from_compiled(document, logic))
    }

    fn from_compiled(document: Document, logic: Option<CompiledDocumentLogic>) -> Self {
        let plan = CompiledPlan::from_compiled(&document, logic);
        Self {
            document: Arc::new(document),
            plan,
        }
    }

    /// 借用底层文档。
    #[must_use]
    #[inline]
    pub fn document(&self) -> &Document {
        &self.document
    }

    /// 克隆底层文档的共享句柄，不复制题面内容。
    #[must_use]
    #[inline]
    pub fn document_arc(&self) -> Arc<Document> {
        Arc::clone(&self.document)
    }

    /// 消费并返回底层文档；共享句柄已被其他调用方持有时才复制一次。
    #[must_use]
    #[inline]
    pub fn into_document(self) -> Document {
        Arc::unwrap_or_clone(self.document)
    }

    /// 复用已编译计划校验提交答案（不分配 `question_states` 与 `path`）。
    ///
    /// # Errors
    /// 答案存在未知/重复题目、不适用答案、类型或范围错误、缺失必填项时，
    /// 返回包含相关题目 ID 的校验错误。
    pub fn validate_answers(
        &self,
        answers: Vec<Answer>,
    ) -> Result<BTreeMap<Uuid, Value>, ValidationError> {
        self.plan
            .validate_answers_only(&self.document, answers)
            .map_err(|error| ValidationError::new(error.question_id, error.to_string()))
    }

    /// 从已存储答案重算规范化答案与题目适用性，跳过路径分配。
    ///
    /// # Errors
    /// 已存答案存在未知/重复题目、不适用答案、类型或范围错误、缺失必填项，
    /// 或路径未到达结束节点时返回领域错误；筛选结束不再是错误。
    pub fn evaluate_stored_states(
        &self,
        answers: &BTreeMap<Uuid, Value>,
    ) -> Result<StoredStates, LogicError> {
        self.plan.evaluate_stored_states(&self.document, answers)
    }

    /// 校验并重建一次提交：一次返回规范化答案、三态与终点结果。
    ///
    /// # Errors
    /// 未知/重复题目、不适用答案、类型或范围错误、缺失必填项，
    /// 或路径未到达任何结束节点时返回可定位的校验错误。
    pub fn evaluate_submission(
        &self,
        answers: Vec<Answer>,
    ) -> Result<EvaluatedResponse, ValidationError> {
        self.plan
            .evaluate_answers(&self.document, answers)
            .map_err(|error| ValidationError::new(error.question_id, error.to_string()))
    }
}

impl Question {
    pub(crate) fn normalize_answer(
        &self,
        value: Option<&Value>,
    ) -> Result<Option<Value>, ValidationError> {
        let id = Some(self.id);
        let Some(value) = value.filter(|value| !value.is_null()) else {
            return Ok(None);
        };
        match &self.kind {
            QuestionKind::SingleChoice { options } => {
                let option_id = parse_option_id(value, self.id)?;
                if !options.iter().any(|option| option.id == option_id) {
                    return Err(ValidationError::new(id, "选项不属于此版本的当前题目"));
                }
                Ok(Some(Value::String(option_id.to_string())))
            }
            QuestionKind::MultipleChoice {
                options,
                min_choices,
                max_choices,
            } => self.normalize_multiple_choice(options, *min_choices, *max_choices, value),
            QuestionKind::Text { max_length } | QuestionKind::Textarea { max_length } => {
                let Value::String(text) = value else {
                    return Err(ValidationError::new(id, "文本答案必须是字符串"));
                };
                // 单遍同时完成长度、空字符、单行换行与全空白判断；
                // 错误优先级保持与原实现一致：长度 → 空字符 → 换行 → 空白视为缺失。
                let single_line = matches!(&self.kind, QuestionKind::Text { .. });
                let mut has_newline = false;
                let mut has_content = false;
                for (index, character) in text.chars().enumerate() {
                    if index == *max_length {
                        return Err(ValidationError::new(
                            id,
                            format!("文本答案最多允许 {max_length} 个 Unicode 字符"),
                        ));
                    }
                    if character == '\0' {
                        return Err(ValidationError::new(id, "文本答案不能包含 U+0000 空字符"));
                    }
                    if single_line
                        && matches!(
                            character,
                            '\n' | '\r'
                                | '\u{000b}'
                                | '\u{000c}'
                                | '\u{0085}'
                                | '\u{2028}'
                                | '\u{2029}'
                        )
                    {
                        has_newline = true;
                    }
                    if !has_content && !character.is_whitespace() {
                        has_content = true;
                    }
                }
                if has_newline {
                    return Err(ValidationError::new(id, "单行文本不能包含换行字符"));
                }
                if has_content {
                    Ok(Some(Value::String(text.clone())))
                } else {
                    Ok(None)
                }
            }
            QuestionKind::Number { integer, min, max } => {
                let number = finite_number(value, self.id)?;
                if *integer && number.fract() != 0.0 {
                    return Err(ValidationError::new(id, "此题只接受整数"));
                }
                if min.is_some_and(|min| number < min) || max.is_some_and(|max| number > max) {
                    return Err(ValidationError::new(id, "数值不在允许范围内"));
                }
                Ok(Some(canonical_number(number)))
            }
            QuestionKind::Rating { min, max, .. } => {
                let number = finite_number(value, self.id)?;
                if number.fract() != 0.0 || number < f64::from(*min) || number > f64::from(*max) {
                    return Err(ValidationError::new(id, "评分必须是允许范围内的整数"));
                }
                Ok(Some(canonical_number(number)))
            }
        }
    }

    /// 校验多选答案的数量、归属与重复，并按选项身份排序返回。
    fn normalize_multiple_choice(
        &self,
        options: &[Choice],
        min_choices: usize,
        max_choices: usize,
        value: &Value,
    ) -> Result<Option<Value>, ValidationError> {
        let id = Some(self.id);
        let Value::Array(values) = value else {
            return Err(ValidationError::new(id, "多选答案必须是选项 ID 数组"));
        };
        if values.is_empty() {
            return Ok(None);
        }
        if values.len() < min_choices || values.len() > max_choices {
            return Err(ValidationError::new(id, "所选数量不符合此题的上下限"));
        }
        let mut selected = BTreeSet::new();
        for value in values {
            let option_id = parse_option_id(value, self.id)?;
            if !options.iter().any(|option| option.id == option_id) {
                return Err(ValidationError::new(id, "选项不属于此版本的当前题目"));
            }
            if !selected.insert(option_id) {
                return Err(ValidationError::new(id, "多选答案不能包含重复选项"));
            }
        }
        Ok(Some(Value::Array(
            selected
                .into_iter()
                .map(|option_id| Value::String(option_id.to_string()))
                .collect(),
        )))
    }
}

fn check_length(
    text: &str,
    maximum: usize,
    question_id: Option<Uuid>,
    name: &str,
) -> Result<(), ValidationError> {
    // 同一遍检查字符数和空字符，越界即返回，不扫描超长输入的剩余内容。
    for (index, character) in text.chars().enumerate() {
        if index == maximum {
            return Err(ValidationError::new(
                question_id,
                format!("{name}最多允许 {maximum} 个 Unicode 字符"),
            ));
        }
        if character == '\0' {
            return Err(ValidationError::new(
                question_id,
                format!("{name}不能包含 U+0000 空字符"),
            ));
        }
    }
    Ok(())
}

fn parse_option_id(value: &Value, question_id: Uuid) -> Result<Uuid, ValidationError> {
    value
        .as_str()
        .and_then(crate::identity::parse)
        .ok_or_else(|| ValidationError::new(Some(question_id), "选项 ID 必须是合法的 UUID 字符串"))
}

pub(crate) fn finite_number(value: &Value, question_id: Uuid) -> Result<f64, ValidationError> {
    value
        .as_f64()
        .filter(|number| number.is_finite())
        .ok_or_else(|| ValidationError::new(Some(question_id), "答案必须是有限的 JSON 数字"))
}

/// 统一正零与负零；IEEE 754 保留负零的符号，但问卷中的两者是相同答案。
pub(crate) fn canonical_zero(number: f64) -> f64 {
    if number == 0.0 { 0.0 } else { number }
}

pub(crate) fn canonical_number(number: f64) -> Value {
    Value::Number(
        Number::from_f64(canonical_zero(number)).expect("BUG: 有限 f64 必然可转换为 JSON Number"),
    )
}

#[cfg(test)]
mod tests {
    use serde_json::{Number, Value, json};
    use uuid::Uuid;

    use super::*;

    fn id(value: u128) -> Uuid {
        Uuid::from_u128(value)
    }

    fn 选项(value: u128) -> Choice {
        Choice {
            id: id(value),
            label: format!("选项 {value}"),
        }
    }

    fn 题目(value: u128, kind: QuestionKind) -> Question {
        Question {
            id: id(value),
            title: format!("题目 {value}"),
            required: false,
            kind,
        }
    }

    fn 数值(answer: Result<Option<Value>, ValidationError>) -> f64 {
        answer
            .expect("答案应通过规范化")
            .expect("应有规范化答案")
            .as_f64()
            .expect("规范化答案应为 JSON 数字")
    }

    #[test]
    fn 长度检查按_unicode_标量计数并携带题目定位() {
        assert!(
            check_length("abcde", 5, None, "文本").is_ok(),
            "精确等于上限应通过"
        );
        let 错误 = check_length("abcdef", 5, None, "文本").unwrap_err();
        assert_eq!(错误.question_id, None);
        assert_eq!(错误.message, "文本最多允许 5 个 Unicode 字符");

        // 中文与四字节 Emoji 都只占一个 Unicode 标量，不按 UTF-8 字节数计算。
        assert!(
            check_length("中", 1, None, "文本").is_ok(),
            "三字节字符仍只算一个标量"
        );
        assert!(
            check_length("😀", 1, None, "文本").is_ok(),
            "四字节字符仍只算一个标量"
        );
        let 错误 = check_length("中文", 1, Some(id(7)), "题目标题").unwrap_err();
        assert_eq!(错误.question_id, Some(id(7)));
        assert_eq!(错误.message, "题目标题最多允许 1 个 Unicode 字符");

        // U+0000 即使没有超长也必须拒绝。
        let 错误 = check_length("a\0b", 10, Some(id(8)), "调查说明").unwrap_err();
        assert_eq!(错误.question_id, Some(id(8)));
        assert_eq!(错误.message, "调查说明不能包含 U+0000 空字符");
    }

    #[test]
    fn 草稿校验拒绝非有限数值边界() {
        for (min, max, 说明) in [
            (Some(f64::INFINITY), None, "正无穷下界"),
            (Some(f64::NEG_INFINITY), None, "负无穷下界"),
            (None, Some(f64::NAN), "NaN 上界"),
        ] {
            let document = Document {
                questions: vec![题目(
                    1,
                    QuestionKind::Number {
                        integer: false,
                        min,
                        max,
                    },
                )],
                ..Document::default()
            };
            let 错误 = document.validate_draft_fields().unwrap_err();
            assert_eq!(错误.question_id, Some(id(1)), "{说明}");
            assert_eq!(错误.message, "数值上下界必须是有限数值", "{说明}");
        }

        let document = Document {
            questions: vec![题目(
                1,
                QuestionKind::Number {
                    integer: true,
                    min: Some(-1.0),
                    max: Some(1.0),
                },
            )],
            ..Document::default()
        };
        assert!(
            document.validate_draft_fields().is_ok(),
            "有限闭区间应通过草稿校验"
        );
    }

    #[test]
    fn 单选答案必须是本题选项的标准_uuid() {
        let 题 = 题目(
            1,
            QuestionKind::SingleChoice {
                options: vec![选项(101), 选项(102)],
            },
        );
        assert_eq!(题.normalize_answer(None).unwrap(), None);
        assert_eq!(题.normalize_answer(Some(&Value::Null)).unwrap(), None);
        assert_eq!(
            题.normalize_answer(Some(&json!(id(101).to_string())))
                .unwrap(),
            Some(Value::String(id(101).to_string()))
        );
        assert_eq!(
            题.normalize_answer(Some(&json!(id(102).to_string().to_uppercase())))
                .unwrap(),
            Some(Value::String(id(102).to_string())),
            "大写十六进制归一为小写身份"
        );

        for (值, 消息) in [
            (json!(101), "选项 ID 必须是合法的 UUID 字符串"),
            (json!("not-a-uuid"), "选项 ID 必须是合法的 UUID 字符串"),
            (
                json!(Uuid::nil().to_string()),
                "选项 ID 必须是合法的 UUID 字符串",
            ),
            (
                json!("00000000-0000-4000-8000-00000000010A"),
                "选项不属于此版本的当前题目",
            ),
            (json!(id(999).to_string()), "选项不属于此版本的当前题目"),
        ] {
            let 错误 = 题.normalize_answer(Some(&值)).unwrap_err();
            assert_eq!(错误.question_id, Some(id(1)));
            assert_eq!(错误.message, 消息, "{值}");
        }
    }

    #[test]
    fn 多选答案校验数量归属重复并排序归一() {
        let 题 = 题目(
            1,
            QuestionKind::MultipleChoice {
                options: vec![选项(101), 选项(102), 选项(103)],
                min_choices: 2,
                max_choices: 2,
            },
        );
        assert_eq!(
            题.normalize_answer(Some(&json!([]))).unwrap(),
            None,
            "空数组视为未回答"
        );

        let 少于 = 题
            .normalize_answer(Some(&json!([id(101).to_string()])))
            .unwrap_err();
        assert_eq!(少于.message, "所选数量不符合此题的上下限");
        let 多于 = 题
            .normalize_answer(Some(&json!([
                id(101).to_string(),
                id(102).to_string(),
                id(103).to_string()
            ])))
            .unwrap_err();
        assert_eq!(多于.message, "所选数量不符合此题的上下限");
        let 重复 = 题
            .normalize_answer(Some(&json!([id(101).to_string(), id(101).to_string()])))
            .unwrap_err();
        assert_eq!(重复.message, "多选答案不能包含重复选项");

        assert_eq!(
            题.normalize_answer(Some(&json!([id(102).to_string(), id(101).to_string()])))
                .unwrap(),
            Some(json!([id(101).to_string(), id(102).to_string()])),
            "多选结果按选项身份排序"
        );

        let 非数组 = 题
            .normalize_answer(Some(&json!(id(101).to_string())))
            .unwrap_err();
        assert_eq!(非数组.message, "多选答案必须是选项 ID 数组");
        let 外来 = 题
            .normalize_answer(Some(&json!([id(101).to_string(), id(999).to_string()])))
            .unwrap_err();
        assert_eq!(外来.message, "选项不属于此版本的当前题目");
        let 非法身份 = 题
            .normalize_answer(Some(&json!([id(101).to_string(), "not-a-uuid"])))
            .unwrap_err();
        assert_eq!(非法身份.message, "选项 ID 必须是合法的 UUID 字符串");
    }

    #[test]
    fn 单行文本拒绝七种换行并保留非空白原文() {
        let 题 = 题目(1, QuestionKind::Text { max_length: 5 });
        assert_eq!(题.normalize_answer(Some(&json!("   "))).unwrap(), None);
        assert_eq!(
            题.normalize_answer(Some(&json!(" 原文 "))).unwrap(),
            Some(json!(" 原文 ")),
            "非空白文本不修剪"
        );

        let 非字符串 = 题.normalize_answer(Some(&json!(1))).unwrap_err();
        assert_eq!(非字符串.message, "文本答案必须是字符串");

        for 换行 in [
            '\n', '\r', '\u{000b}', '\u{000c}', '\u{0085}', '\u{2028}', '\u{2029}',
        ] {
            let 文本 = format!("前{换行}后");
            let 错误 = 题.normalize_answer(Some(&json!(文本))).unwrap_err();
            assert_eq!(
                错误.message, "单行文本不能包含换行字符",
                "换行符 U+{:04X} 应被拦截",
                换行 as u32
            );
        }

        assert_eq!(
            题.normalize_answer(Some(&json!("abcde"))).unwrap(),
            Some(json!("abcde")),
            "精确等于最大长度应通过"
        );
        let 超长 = 题.normalize_answer(Some(&json!("abcdef"))).unwrap_err();
        assert_eq!(超长.message, "文本答案最多允许 5 个 Unicode 字符");
    }

    #[test]
    fn 多行文本允许换行但拒绝空字符() {
        let 题 = 题目(1, QuestionKind::Textarea { max_length: 10 });
        assert_eq!(
            题.normalize_answer(Some(&json!("第一行\n第二行"))).unwrap(),
            Some(json!("第一行\n第二行"))
        );
        let 空字符 = 题.normalize_answer(Some(&json!("a\0b"))).unwrap_err();
        assert_eq!(空字符.message, "文本答案不能包含 U+0000 空字符");
        assert_eq!(
            题.normalize_answer(Some(&json!(" \n\t "))).unwrap(),
            None,
            "纯空白视为未回答"
        );
    }

    #[test]
    fn 数字答案校验有限性整数性闭区间与负零() {
        let 整数题 = 题目(
            1,
            QuestionKind::Number {
                integer: true,
                min: None,
                max: None,
            },
        );
        assert_eq!(数值(整数题.normalize_answer(Some(&json!(2)))), 2.0);
        assert_eq!(
            数值(整数题.normalize_answer(Some(&json!(2.0)))),
            2.0,
            "整数题接受 2.0 这种数学意义上的整数"
        );
        let 非整数 = 整数题.normalize_answer(Some(&json!(2.5))).unwrap_err();
        assert_eq!(非整数.message, "此题只接受整数");
        let 非数字 = 整数题.normalize_answer(Some(&json!("2"))).unwrap_err();
        assert_eq!(非数字.message, "答案必须是有限的 JSON 数字");

        let 区间题 = 题目(
            2,
            QuestionKind::Number {
                integer: false,
                min: Some(-1.5),
                max: Some(2.5),
            },
        );
        assert_eq!(
            数值(区间题.normalize_answer(Some(&json!(-1.5)))),
            -1.5,
            "闭区间下界应通过"
        );
        assert_eq!(
            数值(区间题.normalize_answer(Some(&json!(2.5)))),
            2.5,
            "闭区间上界应通过"
        );
        let 低于 = 区间题.normalize_answer(Some(&json!(-1.6))).unwrap_err();
        assert_eq!(低于.message, "数值不在允许范围内");
        let 高于 = 区间题.normalize_answer(Some(&json!(2.6))).unwrap_err();
        assert_eq!(高于.message, "数值不在允许范围内");

        let 负零 =
            数值(区间题.normalize_answer(Some(&Value::Number(Number::from_f64(-0.0).unwrap()))));
        assert_eq!(负零.to_bits(), 0.0_f64.to_bits(), "负零归一为正零");
    }

    #[test]
    fn 评分答案必须是区间内整数() {
        let 题 = 题目(
            1,
            QuestionKind::Rating {
                min: 1,
                max: 5,
                min_label: "很差".into(),
                max_label: "很好".into(),
            },
        );
        assert_eq!(数值(题.normalize_answer(Some(&json!(1)))), 1.0);
        assert_eq!(数值(题.normalize_answer(Some(&json!(5)))), 5.0);
        for 值 in [json!(0), json!(6), json!(2.5)] {
            let 错误 = 题.normalize_answer(Some(&值)).unwrap_err();
            assert_eq!(错误.message, "评分必须是允许范围内的整数", "{值}");
        }
        let 非数字 = 题.normalize_answer(Some(&json!("3"))).unwrap_err();
        assert_eq!(非数字.message, "答案必须是有限的 JSON 数字");
    }
}
