//! 问卷内容与答案的领域规则，不依赖 HTTP、数据库或当前登录用户。
//!
//! 草稿可以暂时没有标题、题目或完整配置，但始终受容量和身份唯一性限制。
//! 所有文字长度均按 Unicode 标量值（`chars()`）计数，而不是 UTF-8 字节数。
//! 所有题面和文本答案都拒绝 U+0000 空字符，避免文本无法持久化。

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use utoipa::ToSchema;
use uuid::Uuid;

pub use crate::flow::{
    AnswerState, EvaluatedResponse, FlowBranch, FlowDefinition, FlowEvaluator, FlowEvent, FlowNode,
    FlowOutcome, FlowSession, FlowSnapshot,
};
pub use crate::logic::{
    CompiledExpression, LogicError, LogicErrorCode, LogicValue, LogicValueType, compile_expression,
};
pub use crate::logic_document::{
    CompiledDocumentLogic, ComputedVariable, DocumentLogic, DocumentLogicValues, QuestionRule,
    RuleValues,
};
pub use crate::logic_text::{LogicTextError, format_logic_text, parse_logic_text};

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
        use serde::de::Error;

        let Value::Object(mut fields) = Value::deserialize(deserializer)? else {
            return Err(D::Error::custom("题目必须是 JSON 对象"));
        };
        // 不用 flatten：Serde 的 flatten 与 deny_unknown_fields 不能同时生效。
        // 先取出公共字段，其余字段全部交给严格的 QuestionKind，拼错的配置项会被拒绝而不是丢弃。
        let mut common = |name: &'static str| {
            fields
                .remove(name)
                .ok_or_else(|| D::Error::missing_field(name))
        };
        let id = common("id")?;
        let title = common("title")?;
        let required = common("required")?;
        Ok(Self {
            id: crate::identity::deserialize(id).map_err(D::Error::custom)?,
            title: String::deserialize(title).map_err(D::Error::custom)?,
            required: bool::deserialize(required).map_err(D::Error::custom)?,
            kind: QuestionKind::deserialize(Value::Object(fields)).map_err(D::Error::custom)?,
        })
    }
}

/// 六种题型的配置；JSON 使用 `type` 字段和 snake_case 题型名区分。
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
    /// 允许空问卷、空白标题、暂缺选项及暂时矛盾的配置范围；发布前须补全。
    /// 此函数不修改内容，也不会自动生成或修复重复 ID。
    ///
    /// # Errors
    /// 超过容量、文本含空字符、出现空或重复 ID、数值边界非有限时返回可定位的错误。
    pub fn validate_draft(&self) -> Result<(), ValidationError> {
        self.validate_draft_compiled().map(|_| ())
    }

    fn validate_draft_compiled(&self) -> Result<Option<CompiledDocumentLogic>, ValidationError> {
        self.validate_draft_fields()?;
        self.compile_logic()
            .map_err(|error| ValidationError::new(error.question_id, error.to_string()))
    }

    // 独立流程入口先编译规则，再复用字段校验，保持其原有错误优先级。
    pub(crate) fn validate_draft_fields(&self) -> Result<(), ValidationError> {
        check_length(&self.title, MAX_DOCUMENT_TITLE_LENGTH, None, "问卷标题")?;
        check_length(&self.description, MAX_TEXT_LENGTH, None, "调查说明")?;
        check_length(&self.purpose, MAX_TEXT_LENGTH, None, "收集目的")?;
        check_length(&self.thank_you, MAX_THANK_YOU_LENGTH, None, "结束语")?;
        if self.questions.len() > MAX_QUESTIONS {
            return Err(ValidationError::new(None, "每份问卷最多允许 100 道题目"));
        }

        let mut question_ids = HashSet::with_capacity(self.questions.len());
        let mut option_ids = HashSet::new();
        for question in &self.questions {
            let id = Some(question.id);
            if question.id.is_nil() || !question_ids.insert(question.id) {
                return Err(ValidationError::new(id, "题目 ID 不能为空或重复"));
            }
            check_length(&question.title, MAX_QUESTION_TITLE_LENGTH, id, "题目标题")?;
            match &question.kind {
                QuestionKind::SingleChoice { options }
                | QuestionKind::MultipleChoice { options, .. } => {
                    if options.len() > MAX_CHOICES {
                        return Err(ValidationError::new(id, "每道选择题最多允许 50 个选项"));
                    }
                    for option in options {
                        if option.id.is_nil() || !option_ids.insert(option.id) {
                            return Err(ValidationError::new(id, "选项 ID 不能为空或在问卷中重复"));
                        }
                        check_length(&option.label, MAX_CHOICE_LABEL_LENGTH, id, "选项文字")?;
                    }
                    if let QuestionKind::MultipleChoice {
                        min_choices,
                        max_choices,
                        ..
                    } = &question.kind
                        && (*min_choices > MAX_CHOICES || *max_choices > MAX_CHOICES)
                    {
                        return Err(ValidationError::new(id, "选择数量配置不能超过 50"));
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
            }
        }
        Ok(())
    }

    /// 编译文档规则；未配置 `logic` 的文档返回 None。
    ///
    /// # 错误
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
        let logic = self.validate_draft_compiled()?;
        // 筛选结果的保存与账号限次策略尚未确定，暂不发布包含筛选结束的流程。
        if self
            .logic
            .as_ref()
            .and_then(|logic| logic.flow.as_ref())
            .is_some_and(|flow| {
                flow.nodes.iter().any(|node| {
                    matches!(
                        node,
                        FlowNode::End {
                            outcome: FlowOutcome::ScreenOut,
                            ..
                        }
                    )
                })
            })
        {
            return Err(ValidationError::new(
                None,
                "筛选结束的保存与限次策略尚未开放，请使用普通完成节点后再发布",
            ));
        }
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
            match &question.kind {
                QuestionKind::SingleChoice { options }
                | QuestionKind::MultipleChoice { options, .. } => {
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

    /// 为复制出的问卷重新生成所有题目和选项的随机 UUID。
    ///
    /// 除身份外，内容、规则和显示顺序均保持不变；普通保存和排序不要调用。
    pub fn renew_ids(&mut self) {
        let mut question_ids = BTreeMap::new();
        let mut option_ids = BTreeMap::new();
        for question in &mut self.questions {
            let new_id = Uuid::new_v4();
            question_ids.insert(question.id, new_id);
            question.id = new_id;
            match &mut question.kind {
                QuestionKind::SingleChoice { options }
                | QuestionKind::MultipleChoice { options, .. } => {
                    for option in options {
                        let new_id = Uuid::new_v4();
                        option_ids.insert(option.id, new_id);
                        option.id = new_id;
                    }
                }
                QuestionKind::Text { .. }
                | QuestionKind::Textarea { .. }
                | QuestionKind::Number { .. }
                | QuestionKind::Rating { .. } => {}
            }
        }
        if let Some(logic) = &mut self.logic {
            logic.renew_references(&question_ids, &option_ids);
        }
    }

    /// 校验答案并生成可以稳定序列化、用于存储和幂等比较的规范化结果。
    ///
    /// 要求文档满足发布态的结构规则；答案长度按版本内每题保存的 `max_length` 检查。
    /// 题目按 UUID 排序，多选按选项 UUID 排序，
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
        if let Some(logic) = self.validate_publish_compiled()? {
            return FlowEvaluator::from_compiled(self, logic)
                .evaluate_answers(answers)
                .map(|result| result.answers)
                .map_err(|error| ValidationError::new(error.question_id, error.to_string()));
        }
        let questions: HashMap<_, _> = self
            .questions
            .iter()
            .map(|question| (question.id, question))
            .collect();
        let mut seen = HashSet::with_capacity(answers.len().min(MAX_QUESTIONS));
        let mut normalized = BTreeMap::new();
        for answer in answers {
            let id = Some(answer.question_id);
            if !seen.insert(answer.question_id) {
                return Err(ValidationError::new(id, "同一道题目不能重复提交"));
            }
            let question = questions
                .get(&answer.question_id)
                .ok_or_else(|| ValidationError::new(id, "答案包含不属于此问卷版本的题目"))?;
            if let Some(value) = question.normalize_answer(Some(&answer.value))? {
                normalized.insert(question.id, value);
            }
        }
        for question in &self.questions {
            if question.required && !normalized.contains_key(&question.id) {
                return Err(ValidationError::new(Some(question.id), "此题为必填项"));
            }
        }
        Ok(normalized)
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
            } => {
                let Value::Array(values) = value else {
                    return Err(ValidationError::new(id, "多选答案必须是选项 ID 数组"));
                };
                if values.is_empty() {
                    return Ok(None);
                }
                if values.len() < *min_choices || values.len() > *max_choices {
                    return Err(ValidationError::new(id, "所选数量不符合此题的上下限"));
                }
                let allowed: HashSet<_> = options.iter().map(|option| option.id).collect();
                let mut selected = Vec::with_capacity(values.len());
                for value in values {
                    let option_id = parse_option_id(value, self.id)?;
                    if !allowed.contains(&option_id) {
                        return Err(ValidationError::new(id, "选项不属于此版本的当前题目"));
                    }
                    selected.push(option_id);
                }
                selected.sort_unstable();
                if selected.windows(2).any(|pair| pair[0] == pair[1]) {
                    return Err(ValidationError::new(id, "多选答案不能包含重复选项"));
                }
                Ok(Some(Value::Array(
                    selected
                        .into_iter()
                        .map(|option_id| Value::String(option_id.to_string()))
                        .collect(),
                )))
            }
            QuestionKind::Text { max_length } | QuestionKind::Textarea { max_length } => {
                let Value::String(text) = value else {
                    return Err(ValidationError::new(id, "文本答案必须是字符串"));
                };
                check_length(text, *max_length, id, "文本答案")?;
                if matches!(&self.kind, QuestionKind::Text { .. })
                    && text.contains([
                        '\n', '\r', '\u{000b}', '\u{000c}', '\u{0085}', '\u{2028}', '\u{2029}',
                    ])
                {
                    return Err(ValidationError::new(id, "单行文本不能包含换行字符"));
                }
                if text.trim().is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(Value::String(text.clone())))
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
                canonical_number(number, self.id).map(Some)
            }
            QuestionKind::Rating { min, max, .. } => {
                let number = finite_number(value, self.id)?;
                if number.fract() != 0.0 || number < f64::from(*min) || number > f64::from(*max) {
                    return Err(ValidationError::new(id, "评分必须是允许范围内的整数"));
                }
                canonical_number(number, self.id).map(Some)
            }
        }
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
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(|| ValidationError::new(Some(question_id), "选项 ID 必须是合法的 UUID 字符串"))
}

pub(crate) fn finite_number(value: &Value, question_id: Uuid) -> Result<f64, ValidationError> {
    value
        .as_f64()
        .filter(|number| number.is_finite())
        .ok_or_else(|| ValidationError::new(Some(question_id), "答案必须是有限的 JSON 数字"))
}

pub(crate) fn canonical_number(number: f64, question_id: Uuid) -> Result<Value, ValidationError> {
    // IEEE 754 保留负零的符号，但问卷中的正零与负零是相同答案。
    Number::from_f64(if number == 0.0 { 0.0 } else { number })
        .map(Value::Number)
        .ok_or_else(|| ValidationError::new(Some(question_id), "答案必须是有限的 JSON 数字"))
}
