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
    #[must_use]
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
    /// # Errors
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
    /// # Errors
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
        self.check_expression_budget()?;
        let positions: BTreeMap<_, _> = questions
            .iter()
            .enumerate()
            .map(|(index, question)| (question.id, index))
            .collect();
        let (occupied, types) = self.collect_identities(questions)?;
        // 即使规则包为空，也必须检查版本和题目绑定。
        let context = CompilationContext::new(
            &self.dsl_version,
            &self.semantics_version,
            questions,
            &types,
        )?;
        let variables = self.compile_variables(&context)?;
        // 同时就绪的变量按最小 UUID 取出，保持确定性的求值与依赖展开顺序。
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
        let (rules, rule_indices) =
            self.compile_rules(&context, &positions, &question_dependencies)?;
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

    /// 规则包中的表达式逐条检查原始预算，避免编译前消耗过多资源。
    fn check_expression_budget(&self) -> Result<(), LogicError> {
        for expression in self
            .variables
            .iter()
            .map(|variable| &variable.expression)
            .chain(self.rules.iter().flat_map(|rule| {
                [&rule.visible_if, &rule.required_if, &rule.validate]
                    .into_iter()
                    .flatten()
            }))
            .chain(
                self.flow
                    .iter()
                    .flat_map(crate::flow::FlowDefinition::expressions),
            )
        {
            check_raw_budget(Some(expression), 1, &mut 0)?;
        }
        Ok(())
    }

    /// 登记题目、选项与变量身份，返回占用集合及变量声明类型。
    fn collect_identities(
        &self,
        questions: &[Question],
    ) -> Result<(BTreeSet<Uuid>, BTreeMap<Uuid, LogicValueType>), LogicError> {
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
        Ok((occupied, types))
    }

    /// 编译变量表达式并核对声明类型；声明顺序不参与执行顺序。
    fn compile_variables(
        &self,
        context: &CompilationContext,
    ) -> Result<BTreeMap<Uuid, CompiledVariable>, LogicError> {
        let mut variables = BTreeMap::new();
        for (index, variable) in self.variables.iter().enumerate() {
            let path = format!("$.logic.variables[{index}].expression");
            let expression = context
                .compile(&variable.expression)
                .map_err(|failure| failure.relocate(&path))?;
            if expression.value_type() != variable.value_type {
                return Err(error(LogicErrorCode::TypeMismatch, &path, None));
            }
            variables.insert(variable.id, CompiledVariable { expression, path });
        }
        Ok(variables)
    }

    /// 编译题目规则，保持文档声明顺序并保留路径定位。
    fn compile_rules(
        &self,
        context: &CompilationContext,
        positions: &BTreeMap<Uuid, usize>,
        question_dependencies: &BTreeMap<Uuid, BTreeSet<Uuid>>,
    ) -> Result<(Vec<RuleExpressions>, BTreeMap<Uuid, usize>), LogicError> {
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
                let expression = context
                    .compile(raw)
                    .map_err(|failure| failure.relocate(&path))?;
                if expression.value_type() != LogicValueType::Boolean {
                    return Err(error(
                        LogicErrorCode::TypeMismatch,
                        &path,
                        Some(rule.question_id),
                    ));
                }
                let dependencies = expand_dependencies(&expression, question_dependencies);
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
        Ok((rules, rule_indices))
    }

    pub(crate) fn renew_references(
        &mut self,
        questions: &BTreeMap<Uuid, Uuid>,
        options: &BTreeMap<Uuid, Uuid>,
    ) {
        let variables: BTreeMap<_, _> = self
            .variables
            .iter()
            .map(|variable| (variable.id, Uuid::now_v7()))
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
    for key in [
        "expression",
        "left",
        "right",
        "value",
        "fallback",
        "condition",
        "then_branch",
        "else_branch",
        "text",
        "substring",
        "prefix",
    ] {
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
