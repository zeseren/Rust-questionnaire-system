//! 文档规则先完成类型与依赖编译；复制必须重写身份引用，未配置 `logic` 的文档不增加规则字段。

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Number, Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::domain::{Question, QuestionKind};
use crate::logic::{
    CompilationContext, CompiledExpression, LogicError, LogicErrorCode, LogicValue, LogicValueType,
    check_raw_budget, error,
};

/// 可持久化的规则包；表达式结构由编译器严格检查。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DocumentLogic {
    /// 缺省时沿用单页问卷；流程与规则一起冻结到版本中。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<crate::flow::FlowDefinition>,
    /// DSL 格式版本。
    #[schema(value_type = u32)]
    pub dsl_version: Number,
    /// 执行语义版本。
    #[schema(value_type = u32)]
    pub semantics_version: Number,
    /// 计算变量；声明顺序不决定执行顺序。
    pub variables: Vec<ComputedVariable>,
    /// 每道题最多一组规则。
    pub rules: Vec<QuestionRule>,
}

/// 计算变量有独立身份和声明类型，不写入用户答案。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ComputedVariable {
    /// 稳定变量 ID。
    #[serde(deserialize_with = "crate::identity::deserialize")]
    pub id: Uuid,
    /// 期望结果类型；编译器核对表达式实际类型。
    pub value_type: LogicValueType,
    /// 使用 variable 节点引用其他变量。
    pub expression: Value,
}

/// 显示与必填条件只能引用前置答案；约束可以引用当前答案。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct QuestionRule {
    /// 应用规则的题目。
    #[serde(deserialize_with = "crate::identity::deserialize")]
    pub question_id: Uuid,
    /// 缺省时显示。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_if: Option<Value>,
    /// 与题目的静态必填条件取或。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_if: Option<Value>,
    /// 缺省时不添加额外约束。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validate: Option<Value>,
}

#[derive(Debug)]
pub(crate) struct RuleExpressions {
    pub(crate) question_id: Uuid,
    pub(crate) path: String,
    pub(crate) visible_if: Option<CompiledExpression>,
    pub(crate) required_if: Option<CompiledExpression>,
    pub(crate) validate: Option<CompiledExpression>,
}

/// 规则表达式原始求值结果；unknown 的产品处理由流程层负责。
#[derive(Debug, Serialize)]
pub struct RuleValues {
    /// 显示条件的三值结果。
    pub visible_if: Option<LogicValue>,
    /// 动态必填条件的三值结果。
    pub required_if: Option<LogicValue>,
    /// 自定义约束的三值结果。
    pub validate: Option<LogicValue>,
}

/// 在给定规范化上下文上的规则计算结果，不代表填写路径或提交资格。
#[derive(Debug, Serialize)]
pub struct DocumentLogicValues {
    /// 计算变量值，不含原始答案。
    pub variables: BTreeMap<Uuid, LogicValue>,
    /// 配置过规则的题目结果。
    pub rules: BTreeMap<Uuid, RuleValues>,
}

/// 完成拓扑排序、类型检查和前置依赖检查的文档规则。
#[derive(Debug)]
pub struct CompiledDocumentLogic {
    variables: BTreeMap<Uuid, CompiledExpression>,
    variable_paths: BTreeMap<Uuid, String>,
    order: Vec<Uuid>,
    rules: Vec<RuleExpressions>,
    rule_indices: BTreeMap<Uuid, usize>,
    question_dependencies: BTreeMap<Uuid, BTreeSet<Uuid>>,
    pub(crate) flow: Option<crate::flow::CompiledFlow>,
}

impl CompiledDocumentLogic {
    pub(crate) fn rule(&self, id: Uuid) -> Option<&RuleExpressions> {
        self.rule_indices.get(&id).map(|&index| &self.rules[index])
    }

    /// 已编译的计算变量；流程编译按 `variable_order` 逐个展开依赖。
    pub(crate) fn variables(&self) -> &BTreeMap<Uuid, CompiledExpression> {
        &self.variables
    }

    /// 已编译的题目规则，保持文档声明顺序。
    pub(crate) fn rules(&self) -> &[RuleExpressions] {
        &self.rules
    }

    pub(crate) fn dependencies(&self, expression: &CompiledExpression) -> BTreeSet<Uuid> {
        let mut result: BTreeSet<_> = expression.dependencies().collect();
        for id in expression.variable_dependencies() {
            result.extend(&self.question_dependencies[&id]);
        }
        result
    }

    pub(crate) fn evaluate_expression(
        &self,
        expression: &CompiledExpression,
        answers: &BTreeMap<Uuid, Value>,
        variables: &mut BTreeMap<Uuid, LogicValue>,
    ) -> Result<LogicValue, LogicError> {
        // 按实际读取计算变量，短路分支不触发计算；调用方可在同一投影阶段复用缓存。
        expression.evaluate_resolving(answers, &mut |id| {
            self.evaluate_variable(id, answers, variables)
        })
    }

    fn evaluate_variable(
        &self,
        id: Uuid,
        answers: &BTreeMap<Uuid, Value>,
        cache: &mut BTreeMap<Uuid, LogicValue>,
    ) -> Result<LogicValue, LogicError> {
        if let Some(value) = cache.get(&id) {
            return Ok(value.clone());
        }
        let value = self.variables[&id]
            .evaluate_resolving(answers, &mut |dependency| {
                self.evaluate_variable(dependency, answers, cache)
            })
            .map_err(|failure| failure.relocate(&self.variable_paths[&id]))?;
        cache.insert(id, value.clone());
        Ok(value)
    }
    /// 同时可执行的变量按 UUID 排序，声明重排不改变计算顺序。
    pub fn variable_order(&self) -> &[Uuid] {
        &self.order
    }

    /// 对已规范化且已排除不适用答案的上下文计算规则；不代替流程或答案校验。
    ///
    /// # 错误
    /// 类型错误、无效答案、除零和数值溢出均返回领域错误。
    pub fn evaluate(
        &self,
        answers: &BTreeMap<Uuid, Value>,
    ) -> Result<DocumentLogicValues, LogicError> {
        let mut variables = BTreeMap::new();
        for id in &self.order {
            variables.insert(
                *id,
                self.variables[id]
                    .evaluate_with_variables(answers, &variables)
                    .map_err(|failure| failure.relocate(&self.variable_paths[id]))?,
            );
        }
        let mut rules = BTreeMap::new();
        for rule in &self.rules {
            let eval = |expression: &Option<CompiledExpression>, field| {
                expression
                    .as_ref()
                    .map(|expression| {
                        expression
                            .evaluate_with_variables(answers, &variables)
                            .map_err(|failure| {
                                failure.relocate(&format!("{}.{}", rule.path, field))
                            })
                    })
                    .transpose()
            };
            rules.insert(
                rule.question_id,
                RuleValues {
                    visible_if: eval(&rule.visible_if, "visible_if")?,
                    required_if: eval(&rule.required_if, "required_if")?,
                    validate: eval(&rule.validate, "validate")?,
                },
            );
        }
        Ok(DocumentLogicValues { variables, rules })
    }
}

impl DocumentLogic {
    /// 检查规则总容量、引用、类型、变量循环和间接前向依赖。
    ///
    /// # 错误
    /// 不接受未知版本、重复身份、空规则、类型错误、环或非法题目顺序。
    pub fn compile(&self, questions: &[Question]) -> Result<CompiledDocumentLogic, LogicError> {
        if self.variables.len() > 100 || self.rules.len() > 100 {
            return Err(error(LogicErrorCode::LimitExceeded, "$.logic", None));
        }
        let mut total = 0;
        let mut bytes = 0;
        for expression in self
            .variables
            .iter()
            .map(|variable| &variable.expression)
            .chain(self.rules.iter().flat_map(|rule| {
                [&rule.visible_if, &rule.required_if, &rule.validate]
                    .into_iter()
                    .flatten()
            }))
            .chain(self.flow.iter().flat_map(|flow| flow.expressions()))
        {
            let mut count = 0;
            check_raw_budget(Some(expression), 1, &mut count)?;
            total += count;
            bytes += serialized_bytes(expression);
            if total > 4096 || bytes > 262_144 {
                return Err(error(LogicErrorCode::LimitExceeded, "$.logic", None));
            }
        }
        let positions: BTreeMap<_, _> = questions
            .iter()
            .enumerate()
            .map(|(index, question)| (question.id, index))
            .collect();
        let mut occupied: BTreeSet<_> = questions.iter().map(|question| question.id).collect();
        for question in questions {
            if let QuestionKind::SingleChoice { options }
            | QuestionKind::MultipleChoice { options, .. } = &question.kind
            {
                occupied.extend(options.iter().map(|option| option.id));
            }
        }
        let mut types = BTreeMap::new();
        for variable in &self.variables {
            if variable.id.is_nil() || !occupied.insert(variable.id) {
                return Err(error(
                    LogicErrorCode::InvalidReference,
                    "$.logic.variables",
                    None,
                ));
            }
            types.insert(variable.id, variable.value_type);
        }
        // 即使规则包为空，也必须检查版本和题目绑定。
        let context = CompilationContext::new(
            &self.dsl_version,
            &self.semantics_version,
            questions,
            &types,
        )?;
        let compile = |expression: &Value, path: &str| {
            context
                .compile(expression)
                .map_err(|failure| failure.relocate(path))
        };
        let mut variables = BTreeMap::new();
        let mut variable_paths = BTreeMap::new();
        for (index, variable) in self.variables.iter().enumerate() {
            let path = format!("$.logic.variables[{index}].expression");
            let expression = compile(&variable.expression, &path)?;
            if expression.value_type() != variable.value_type {
                return Err(error(LogicErrorCode::TypeMismatch, &path, None));
            }
            variables.insert(variable.id, expression);
            variable_paths.insert(variable.id, path);
        }
        let mut order = Vec::with_capacity(variables.len());
        let mut question_dependencies: BTreeMap<Uuid, BTreeSet<Uuid>> = BTreeMap::new();
        while order.len() < variables.len() {
            let next = variables.iter().find(|(id, expression)| {
                !question_dependencies.contains_key(id)
                    && expression
                        .variable_dependencies()
                        .all(|dependency| question_dependencies.contains_key(&dependency))
            });
            let Some((id, expression)) = next else {
                return Err(error(
                    LogicErrorCode::DependencyCycle,
                    "$.logic.variables",
                    None,
                ));
            };
            let mut dependencies: BTreeSet<_> = expression.dependencies().collect();
            for dependency in expression.variable_dependencies() {
                dependencies.extend(&question_dependencies[&dependency]);
            }
            question_dependencies.insert(*id, dependencies);
            order.push(*id);
        }
        let mut seen = BTreeSet::new();
        let mut rules = Vec::with_capacity(self.rules.len());
        let mut rule_indices = BTreeMap::new();
        for (index, rule) in self.rules.iter().enumerate() {
            let path = format!("$.logic.rules[{index}]");
            let position = positions.get(&rule.question_id).ok_or_else(|| {
                error(
                    LogicErrorCode::InvalidReference,
                    &path,
                    Some(rule.question_id),
                )
            })?;
            if !seen.insert(rule.question_id) {
                return Err(error(
                    LogicErrorCode::InvalidReference,
                    &path,
                    Some(rule.question_id),
                ));
            }
            if rule.visible_if.is_none() && rule.required_if.is_none() && rule.validate.is_none() {
                return Err(error(
                    LogicErrorCode::InvalidSyntax,
                    &path,
                    Some(rule.question_id),
                ));
            }
            let compile_rule = |raw: &Option<Value>,
                                field: &str,
                                allow_self: bool|
             -> Result<Option<CompiledExpression>, LogicError> {
                let Some(raw) = raw else {
                    return Ok(None);
                };
                let path = format!("{path}.{field}");
                let expression = compile(raw, &path)?;
                if expression.value_type() != LogicValueType::Boolean {
                    return Err(error(
                        LogicErrorCode::TypeMismatch,
                        &path,
                        Some(rule.question_id),
                    ));
                }
                let mut dependencies: BTreeSet<_> = expression.dependencies().collect();
                for dependency in expression.variable_dependencies() {
                    dependencies.extend(&question_dependencies[&dependency]);
                }
                if dependencies.iter().any(|id| {
                    positions[id] > *position || (!allow_self && positions[id] == *position)
                }) {
                    return Err(error(
                        LogicErrorCode::ForwardReference,
                        &path,
                        Some(rule.question_id),
                    ));
                }
                Ok(Some(expression))
            };
            rule_indices.insert(rule.question_id, rules.len());
            rules.push(RuleExpressions {
                question_id: rule.question_id,
                visible_if: compile_rule(&rule.visible_if, "visible_if", false)?,
                required_if: compile_rule(&rule.required_if, "required_if", false)?,
                validate: compile_rule(&rule.validate, "validate", true)?,
                path,
            });
        }
        let mut compiled = CompiledDocumentLogic {
            variables,
            variable_paths,
            order,
            rules,
            rule_indices,
            question_dependencies,
            flow: None,
        };
        if let Some(flow) = &self.flow {
            // 题目、选项与变量身份已在上面登记，流程节点身份直接续用同一集合。
            compiled.flow = Some(crate::flow::compile_flow(
                flow, occupied, &compiled, questions, &context,
            )?);
        }
        Ok(compiled)
    }

    pub(crate) fn renew_references(
        &mut self,
        questions: &BTreeMap<Uuid, Uuid>,
        options: &BTreeMap<Uuid, Uuid>,
    ) {
        let variables: BTreeMap<_, _> = self
            .variables
            .iter()
            .map(|variable| (variable.id, Uuid::new_v4()))
            .collect();
        for variable in &mut self.variables {
            variable.id = variables[&variable.id];
            rewrite(&mut variable.expression, questions, options, &variables);
        }
        for rule in &mut self.rules {
            if let Some(id) = questions.get(&rule.question_id) {
                rule.question_id = *id;
            }
            for expression in [
                &mut rule.visible_if,
                &mut rule.required_if,
                &mut rule.validate,
            ]
            .into_iter()
            .flatten()
            {
                rewrite(expression, questions, options, &variables);
            }
        }
        if let Some(flow) = &mut self.flow {
            flow.renew_ids(questions, options, &variables);
        }
    }
}

/// 直接统计 JSON 序列化后的 UTF-8 字节数，不分配临时 String。
fn serialized_bytes(value: &Value) -> usize {
    struct ByteCounter(usize);
    impl std::io::Write for ByteCounter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0 += buf.len();
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = ByteCounter(0);
    serde_json::to_writer(&mut counter, value).expect("内存字节计数写入不应失败");
    counter.0
}

pub(crate) fn rewrite(
    expression: &mut Value,
    questions: &BTreeMap<Uuid, Uuid>,
    options: &BTreeMap<Uuid, Uuid>,
    variables: &BTreeMap<Uuid, Uuid>,
) {
    let Value::Object(fields) = expression else {
        return;
    };
    for (key, ids) in [
        ("question_id", questions),
        ("option_id", options),
        ("variable_id", variables),
    ] {
        if let Some(value) = fields.get_mut(key)
            && let Some(id) = value
                .as_str()
                .and_then(|value| Uuid::parse_str(value).ok())
                .and_then(|id| ids.get(&id))
        {
            *value = json!(id);
        }
    }
    if let Some(Value::Array(values)) = fields.get_mut("option_ids") {
        for value in values {
            if let Some(id) = value
                .as_str()
                .and_then(|value| Uuid::parse_str(value).ok())
                .and_then(|id| options.get(&id))
            {
                *value = json!(id);
            }
        }
    }
    for key in ["expression", "left", "right", "value", "fallback"] {
        if let Some(value) = fields.get_mut(key) {
            rewrite(value, questions, options, variables);
        }
    }
    if let Some(Value::Array(values)) = fields.get_mut("expressions") {
        for value in values {
            rewrite(value, questions, options, variables);
        }
    }
}
