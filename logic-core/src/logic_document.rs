//! 文档规则先完成类型与依赖编译；复制必须重写身份引用，未配置 `logic` 的文档不增加规则字段。

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Number, Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::domain::{MAX_QUESTIONS, Question};
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

/// 计算变量的编译结果与来源路径绑在一起存储，避免两张平行表。
#[derive(Debug)]
struct CompiledVariable {
    expression: CompiledExpression,
    path: String,
}

/// 完成拓扑排序、类型检查和前置依赖检查的文档规则。
#[derive(Debug)]
pub struct CompiledDocumentLogic {
    variables: BTreeMap<Uuid, CompiledVariable>,
    order: Vec<Uuid>,
    rules: Vec<RuleExpressions>,
    rule_indices: BTreeMap<Uuid, usize>,
    question_dependencies: BTreeMap<Uuid, BTreeSet<Uuid>>,
    pub(crate) flow: Option<crate::flow::CompiledFlow>,
}

impl CompiledDocumentLogic {
    pub(crate) fn empty() -> Self {
        Self {
            variables: BTreeMap::new(),
            order: Vec::new(),
            rules: Vec::new(),
            rule_indices: BTreeMap::new(),
            question_dependencies: BTreeMap::new(),
            flow: None,
        }
    }

    pub(crate) fn rule(&self, id: Uuid) -> Option<&RuleExpressions> {
        self.rule_indices.get(&id).map(|&index| &self.rules[index])
    }

    /// 已编译的计算变量表达式；流程编译按 `variable_order` 逐个展开依赖。
    pub(crate) fn variable_expression(&self, id: Uuid) -> &CompiledExpression {
        &self.variables[&id].expression
    }

    /// 已编译的题目规则，保持文档声明顺序。
    pub(crate) fn rules(&self) -> &[RuleExpressions] {
        &self.rules
    }

    pub(crate) fn dependencies(&self, expression: &CompiledExpression) -> BTreeSet<Uuid> {
        expand_dependencies(expression, &self.question_dependencies)
    }

    /// 当某题答案因校验失败被回滚时，仅失效传递依赖该题的计算变量缓存。
    pub(crate) fn evict_dependent_variables(
        &self,
        variables: &mut BTreeMap<Uuid, LogicValue>,
        question_id: Uuid,
    ) {
        variables.retain(|id, _| !self.question_dependencies[id].contains(&question_id));
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
        let variable = &self.variables[&id];
        let value = variable
            .expression
            .evaluate_resolving(answers, &mut |dependency| {
                self.evaluate_variable(dependency, answers, cache)
            })
            .map_err(|failure| failure.relocate(&variable.path))?;
        cache.insert(id, value.clone());
        Ok(value)
    }
    /// 同时可执行的变量按 UUID 排序，声明重排不改变计算顺序。
    pub fn variable_order(&self) -> &[Uuid] {
        &self.order
    }

    /// 取出文档实际生效的流程：配置了流程时移出编译产物，否则构建隐式单页流程。
    pub(crate) fn into_flow(mut self, questions: &[Question]) -> (Self, crate::flow::CompiledFlow) {
        let flow = self
            .flow
            .take()
            .unwrap_or_else(|| crate::flow::CompiledFlow::implicit(questions));
        (self, flow)
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
            let variable = &self.variables[id];
            variables.insert(
                *id,
                variable
                    .expression
                    .evaluate_with_variables(answers, &variables)
                    .map_err(|failure| failure.relocate(&variable.path))?,
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

/// 表达式的直接题目依赖与计算变量展开的题目依赖合并为一份集合。
fn expand_dependencies(
    expression: &CompiledExpression,
    question_dependencies: &BTreeMap<Uuid, BTreeSet<Uuid>>,
) -> BTreeSet<Uuid> {
    let mut dependencies: BTreeSet<_> = expression.dependencies().collect();
    for id in expression.variable_dependencies() {
        dependencies.extend(&question_dependencies[&id]);
    }
    dependencies
}

impl DocumentLogic {
    /// 检查规则总容量、引用、类型、变量循环和间接前向依赖。
    ///
    /// # 错误
    /// 不接受未知版本、重复身份、空规则、类型错误、环或非法题目顺序。
    pub fn compile(&self, questions: &[Question]) -> Result<CompiledDocumentLogic, LogicError> {
        if self.variables.len() > MAX_QUESTIONS || self.rules.len() > MAX_QUESTIONS {
            return Err(error(LogicErrorCode::LimitExceeded, "$.logic", None));
        }
        if self.variables.is_empty() && self.rules.is_empty() && self.flow.is_none() {
            // 空规则包同样要经过版本与题目绑定检查，不能静默放行。
            CompilationContext::new(
                &self.dsl_version,
                &self.semantics_version,
                questions,
                &BTreeMap::new(),
            )?;
            return Ok(CompiledDocumentLogic::empty());
        }
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
            check_raw_budget(Some(expression), 1, &mut 0)?;
        }
        let positions: BTreeMap<_, _> = questions
            .iter()
            .enumerate()
            .map(|(index, question)| (question.id, index))
            .collect();
        let mut occupied: BTreeSet<_> = questions.iter().map(|question| question.id).collect();
        for question in questions {
            if let Some(options) = question.kind.options() {
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
        for (index, variable) in self.variables.iter().enumerate() {
            let path = format!("$.logic.variables[{index}].expression");
            let expression = compile(&variable.expression, &path)?;
            if expression.value_type() != variable.value_type {
                return Err(error(LogicErrorCode::TypeMismatch, &path, None));
            }
            variables.insert(variable.id, CompiledVariable { expression, path });
        }
        // 计算变量按拓扑序展开：入度只统计尚未就绪的直接变量依赖，
        // 就绪集合每次取最小 UUID，保持与逐轮全表扫描完全一致的确定性顺序。
        let mut indegree: BTreeMap<Uuid, usize> = variables
            .iter()
            .map(|(id, variable)| (*id, variable.expression.variable_dependencies().count()))
            .collect();
        let mut dependents: BTreeMap<Uuid, Vec<Uuid>> = BTreeMap::new();
        for (id, variable) in &variables {
            for dependency in variable.expression.variable_dependencies() {
                dependents.entry(dependency).or_default().push(*id);
            }
        }
        let mut ready: BTreeSet<Uuid> = indegree
            .iter()
            .filter(|(_, count)| **count == 0)
            .map(|(id, _)| *id)
            .collect();
        let mut order = Vec::with_capacity(variables.len());
        let mut question_dependencies: BTreeMap<Uuid, BTreeSet<Uuid>> = BTreeMap::new();
        while let Some(id) = ready.pop_first() {
            let variable = &variables[&id];
            let dependencies = expand_dependencies(&variable.expression, &question_dependencies);
            question_dependencies.insert(id, dependencies);
            order.push(id);
            if let Some(children) = dependents.get(&id) {
                for child in children {
                    let remaining = indegree.get_mut(child).expect("依赖边只指向已声明的变量");
                    *remaining -= 1;
                    if *remaining == 0 {
                        ready.insert(*child);
                    }
                }
            }
        }
        if order.len() != variables.len() {
            return Err(error(
                LogicErrorCode::DependencyCycle,
                "$.logic.variables",
                None,
            ));
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
                let dependencies = expand_dependencies(&expression, &question_dependencies);
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
                .and_then(crate::identity::parse)
                .and_then(|id| ids.get(&id))
        {
            *value = json!(id);
        }
    }
    if let Some(Value::Array(values)) = fields.get_mut("option_ids") {
        for value in values {
            if let Some(id) = value
                .as_str()
                .and_then(crate::identity::parse)
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::{Number, Value, json};
    use uuid::Uuid;

    use super::*;
    use crate::domain::{Question, QuestionKind};

    fn id(value: u128) -> Uuid {
        Uuid::from_u128(value)
    }

    fn 版本() -> Number {
        Number::from(1_u32)
    }

    fn 数字(value: &str) -> Value {
        json!({"op": "number", "value": value})
    }

    fn 变量(value: u128, value_type: LogicValueType, expression: Value) -> ComputedVariable {
        ComputedVariable {
            id: id(value),
            value_type,
            expression,
        }
    }

    fn 逻辑(variables: Vec<ComputedVariable>, rules: Vec<QuestionRule>) -> DocumentLogic {
        DocumentLogic {
            flow: None,
            dsl_version: 版本(),
            semantics_version: 版本(),
            variables,
            rules,
        }
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

    #[test]
    fn 空规则包不产生变量规则与流程() {
        let 空 = CompiledDocumentLogic::empty();
        assert!(空.variable_order().is_empty());
        assert!(空.rules().is_empty());
        assert!(空.flow.is_none());

        let 值 = 空.evaluate(&BTreeMap::new()).unwrap();
        assert!(值.variables.is_empty());
        assert!(值.rules.is_empty());
    }

    #[test]
    fn 同层变量按身份升序() {
        let logic = 逻辑(
            vec![
                变量(303, LogicValueType::Number, 数字("3")),
                变量(301, LogicValueType::Number, 数字("1")),
                变量(302, LogicValueType::Number, 数字("2")),
            ],
            Vec::new(),
        );
        let 编译结果 = logic.compile(&[]).unwrap();
        assert_eq!(
            编译结果.variable_order(),
            [id(301), id(302), id(303)].as_slice(),
            "同层无关变量按 UUID 升序，声明顺序不参与计算"
        );
    }

    #[test]
    fn 重写覆盖全部表达式引用路径并保留未映射身份() {
        let questions = BTreeMap::from([(id(1), id(11)), (id(2), id(12))]);
        let options = BTreeMap::from([(id(101), id(201)), (id(102), id(202))]);
        let variables = BTreeMap::from([(id(301), id(401)), (id(302), id(402))]);
        let mut expression = json!({
            "op": "all",
            "expressions": [
                {"op": "answer", "question_id": id(1).to_string()},
                {"op": "answered", "question_id": id(2).to_string()},
                {"op": "variable", "variable_id": id(301).to_string()},
                {"op": "selected", "question_id": id(1).to_string(), "option_id": id(101).to_string()},
                {"op": "contains_any", "question_id": id(1).to_string(), "option_ids": [id(101).to_string(), id(102).to_string(), id(999).to_string()]},
                {"op": "contains_all", "question_id": id(1).to_string(), "option_ids": [id(102).to_string()]},
                {"op": "not", "expression": {"op": "answer", "question_id": id(1).to_string()}},
                {"op": "any", "expressions": [{"op": "answered", "question_id": id(2).to_string()}]},
                {"op": "compare", "comparison": "eq", "left": {"op": "answer", "question_id": id(1).to_string()}, "right": {"op": "number", "value": "1"}},
                {"op": "coalesce", "value": {"op": "answer", "question_id": id(2).to_string()}, "fallback": {"op": "number", "value": "0"}},
                {"op": "length", "value": {"op": "answer", "question_id": id(1).to_string()}},
                {"op": "arithmetic", "operator": "add", "left": {"op": "answer", "question_id": id(1).to_string()}, "right": {"op": "variable", "variable_id": id(302).to_string()}},
                {"op": "answer", "question_id": id(999).to_string()},
            ],
        });
        rewrite(&mut expression, &questions, &options, &variables);
        let items = expression["expressions"]
            .as_array()
            .expect("表达式数组应保留");

        assert_eq!(items[0]["question_id"], json!(id(11)), "answer 题目引用");
        assert_eq!(items[1]["question_id"], json!(id(12)), "answered 题目引用");
        assert_eq!(items[2]["variable_id"], json!(id(401)), "variable 变量引用");
        assert_eq!(items[3]["question_id"], json!(id(11)), "selected 题目引用");
        assert_eq!(items[3]["option_id"], json!(id(201)), "selected 选项引用");
        assert_eq!(
            items[4]["option_ids"],
            json!([id(201), id(202), id(999)]),
            "contains_any 映射已配选项并保留未映射选项"
        );
        assert_eq!(
            items[5]["option_ids"],
            json!([id(202)]),
            "contains_all 选项引用"
        );
        assert_eq!(
            items[6]["expression"]["question_id"],
            json!(id(11)),
            "not 子表达式"
        );
        assert_eq!(
            items[7]["expressions"][0]["question_id"],
            json!(id(12)),
            "any 子表达式"
        );
        assert_eq!(
            items[8]["left"]["question_id"],
            json!(id(11)),
            "compare 左侧引用"
        );
        assert_eq!(
            items[9]["value"]["question_id"],
            json!(id(12)),
            "coalesce 首值引用"
        );
        assert_eq!(
            items[10]["value"]["question_id"],
            json!(id(11)),
            "length 参数引用"
        );
        assert_eq!(
            items[11]["left"]["question_id"],
            json!(id(11)),
            "arithmetic 左侧引用"
        );
        assert_eq!(
            items[11]["right"]["variable_id"],
            json!(id(402)),
            "arithmetic 右侧变量引用"
        );
        assert_eq!(
            items[12]["question_id"],
            json!(id(999)),
            "未映射题目身份保持原样"
        );
    }

    #[test]
    fn 变量与规则数量遵守题目上限边界() {
        let 变量超限: Vec<_> = (0..=MAX_QUESTIONS)
            .map(|index| 变量(1000 + index as u128, LogicValueType::Number, 数字("1")))
            .collect();
        let 错误 = 逻辑(变量超限, Vec::new()).compile(&[]).unwrap_err();
        assert_eq!(错误.code, LogicErrorCode::LimitExceeded);
        assert_eq!(错误.path, "$.logic");
        assert_eq!(错误.question_id, None);

        let 规则超限: Vec<_> = (0..=MAX_QUESTIONS)
            .map(|index| QuestionRule {
                question_id: id(2000 + index as u128),
                visible_if: Some(json!({"op": "boolean", "value": true})),
                required_if: None,
                validate: None,
            })
            .collect();
        let 错误 = 逻辑(Vec::new(), 规则超限).compile(&[]).unwrap_err();
        assert_eq!(错误.code, LogicErrorCode::LimitExceeded);
        assert_eq!(错误.path, "$.logic");
        assert_eq!(错误.question_id, None);

        // 恰好 MAX_QUESTIONS 个变量与规则仍可编译，确认拒绝只发生在越过上限之后。
        let questions: Vec<_> = (0..MAX_QUESTIONS)
            .map(|index| 数字题(index as u128 + 1))
            .collect();
        let 变量饱和: Vec<_> = (0..MAX_QUESTIONS)
            .map(|index| 变量(1000 + index as u128, LogicValueType::Number, 数字("1")))
            .collect();
        let 规则饱和: Vec<_> = (0..MAX_QUESTIONS)
            .map(|index| QuestionRule {
                question_id: id(index as u128 + 1),
                visible_if: Some(json!({"op": "boolean", "value": true})),
                required_if: None,
                validate: None,
            })
            .collect();
        let 编译结果 = 逻辑(变量饱和, 规则饱和)
            .compile(&questions)
            .expect("恰好达到上限的变量与规则应通过编译");
        assert_eq!(编译结果.variable_order().len(), MAX_QUESTIONS);
        assert_eq!(编译结果.rules().len(), MAX_QUESTIONS);
    }

    #[test]
    fn 变量身份必须唯一() {
        let 重复 = 逻辑(
            vec![
                变量(301, LogicValueType::Number, 数字("1")),
                变量(301, LogicValueType::Number, 数字("2")),
            ],
            Vec::new(),
        );
        let 错误 = 重复.compile(&[]).unwrap_err();
        assert_eq!(错误.code, LogicErrorCode::InvalidReference);
        assert_eq!(错误.path, "$.logic.variables");
        assert_eq!(错误.question_id, None);
    }

    #[test]
    fn 规则必须指向已知题目且至少配置一个字段() {
        let questions = [数字题(1)];
        let 未知题目 = 逻辑(
            Vec::new(),
            vec![QuestionRule {
                question_id: id(999),
                visible_if: Some(json!({"op": "boolean", "value": true})),
                required_if: None,
                validate: None,
            }],
        );
        let 错误 = 未知题目.compile(&questions).unwrap_err();
        assert_eq!(错误.code, LogicErrorCode::InvalidReference);
        assert_eq!(错误.path, "$.logic.rules[0]");
        assert_eq!(错误.question_id, Some(id(999)));

        let 空规则 = 逻辑(
            Vec::new(),
            vec![QuestionRule {
                question_id: id(1),
                visible_if: None,
                required_if: None,
                validate: None,
            }],
        );
        let 错误 = 空规则.compile(&questions).unwrap_err();
        assert_eq!(错误.code, LogicErrorCode::InvalidSyntax);
        assert_eq!(错误.path, "$.logic.rules[0]");
        assert_eq!(错误.question_id, Some(id(1)));
    }
}
