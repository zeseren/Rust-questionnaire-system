//! 文档规则按依赖顺序编译与计算；规则引用的答案必须在使用位置可获得。
import { checkExpressionBudget, compileExpression, LogicError } from './logic.ts'
import { copy } from './json.ts'
import type { CompiledExpression, LogicValue, LogicValueType } from './logic.ts'
import type { Question, SurveyDocument } from './types'

export interface ComputedVariable {
  id: string
  value_type: LogicValueType
  expression: unknown
}
export interface QuestionRule {
  question_id: string
  visible_if?: unknown
  required_if?: unknown
  validate?: unknown
}
export interface DocumentLogic {
  dsl_version: number
  semantics_version: number
  variables: ComputedVariable[]
  rules: QuestionRule[]
}
export interface DocumentLogicValues {
  variables: Record<string, LogicValue>
  rules: Record<
    string,
    { visible_if: LogicValue | null; required_if: LogicValue | null; validate: LogicValue | null }
  >
}
export interface CompiledDocumentLogic {
  readonly variableOrder: readonly string[]
  /** 输入须已规范化并排除不适用答案；结果不代表填写路径或提交资格。 */
  evaluate(answers: Readonly<Record<string, unknown>>): DocumentLogicValues
}

function record(
  raw: unknown,
  required: string[],
  optional: string[] = [],
): Record<string, unknown> {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw))
    throw new LogicError('invalid_syntax', '$.logic')
  const value = raw as Record<string, unknown>
  if (
    required.some((key) => !Object.hasOwn(value, key)) ||
    Object.keys(value).some((key) => !required.includes(key) && !optional.includes(key))
  )
    throw new LogicError('invalid_syntax', '$.logic')
  return value
}
function id(raw: unknown, path: string): string {
  if (
    typeof raw !== 'string' ||
    !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(raw) ||
    raw === '00000000-0000-0000-0000-000000000000'
  )
    throw new LogicError('invalid_reference', path)
  return raw
}

/** 对 JSON 边界做结构检查，保留 AST 原文供编译器验证。 */
function parseLogic(raw: unknown): DocumentLogic {
  const logic = record(raw, ['dsl_version', 'semantics_version', 'variables', 'rules'])
  if (!Array.isArray(logic.variables) || !Array.isArray(logic.rules))
    throw new LogicError('invalid_syntax', '$.logic')
  if (logic.variables.length > 100 || logic.rules.length > 100)
    throw new LogicError('limit_exceeded', '$.logic')
  const variables = logic.variables.map((raw) => {
    const variable = record(raw, ['id', 'value_type', 'expression'])
    if (!['boolean', 'number', 'text', 'choice', 'choices'].includes(String(variable.value_type)))
      throw new LogicError('invalid_syntax', '$.logic')
    return {
      id: id(variable.id, '$.logic.variables'),
      value_type: variable.value_type as LogicValueType,
      expression: variable.expression,
    }
  })
  const rules = logic.rules.map((raw) => {
    const rule = record(raw, ['question_id'], ['visible_if', 'required_if', 'validate'])
    return {
      question_id: id(rule.question_id, '$.logic.rules'),
      visible_if: rule.visible_if,
      required_if: rule.required_if,
      validate: rule.validate,
    }
  })
  // 版本的数值与范围由表达式编译入口统一检查。
  return {
    dsl_version: logic.dsl_version as number,
    semantics_version: logic.semantics_version as number,
    variables,
    rules,
  }
}

/** 检查总容量、变量环和间接前向引用，返回与原始配置隔离的编译结果。 */
export function compileDocumentLogic(
  raw: unknown,
  questions: readonly Question[],
): CompiledDocumentLogic {
  const logic = parseLogic(raw)
  let total = 0,
    bytes = 0
  for (const expression of [
    ...logic.variables.map((variable) => variable.expression),
    ...logic.rules.flatMap((rule) =>
      [rule.visible_if, rule.required_if, rule.validate].filter((value) => value != null),
    ),
  ]) {
    const budget = { count: 0 }
    checkExpressionBudget(expression, 1, budget)
    total += budget.count
    // 预算按紧凑 JSON 的 UTF-8 字节计算，与后端一致。
    bytes += expressionBytes(expression)
    if (total > 4096 || bytes > 262144) throw new LogicError('limit_exceeded', '$.logic')
  }
  const positions = new Map(questions.map((question, index) => [question.id, index]))
  const occupied = new Set(
    questions.flatMap((question) => [
      question.id,
      ...('options' in question ? question.options.map((option) => option.id) : []),
    ]),
  )
  const types = new Map<string, LogicValueType>()
  for (const variable of logic.variables) {
    if (occupied.has(variable.id)) throw new LogicError('invalid_reference', '$.logic.variables')
    occupied.add(variable.id)
    types.set(variable.id, variable.value_type)
  }
  const compile = (expression: unknown, path: string) => {
    const relocate = (error: unknown): never => {
      if (!(error instanceof LogicError)) throw error
      throw new LogicError(error.code, error.path.replace('$.expression', path), error.question_id)
    }
    try {
      const compiled = compileExpression(
        { dsl_version: logic.dsl_version, semantics_version: logic.semantics_version, expression },
        questions,
        types,
      )
      return {
        ...compiled,
        evaluate(
          answers: Readonly<Record<string, unknown>>,
          variables?: ReadonlyMap<string, LogicValue>,
        ) {
          try {
            return compiled.evaluate(answers, variables)
          } catch (error) {
            return relocate(error)
          }
        },
      }
    } catch (error) {
      return relocate(error)
    }
  }
  compile({ op: 'boolean', value: true }, '$.logic')
  const variables = new Map<string, CompiledExpression>()
  logic.variables.forEach((variable, index) => {
    const path = `$.logic.variables[${index}].expression`
    const expression = compile(variable.expression, path)
    if (expression.valueType !== variable.value_type) throw new LogicError('type_mismatch', path)
    variables.set(variable.id, expression)
  })
  const order: string[] = []
  const questionDependencies = new Map<string, Set<string>>()
  while (order.length < variables.size) {
    const next = [...variables.keys()]
      .sort()
      .find(
        (id) =>
          !questionDependencies.has(id) &&
          variables
            .get(id)!
            .variableDependencies.every((dependency) => questionDependencies.has(dependency)),
      )
    if (!next) throw new LogicError('dependency_cycle', '$.logic.variables')
    const expression = variables.get(next)!
    const dependencies = new Set(expression.dependencies)
    for (const dependency of expression.variableDependencies)
      for (const question of questionDependencies.get(dependency)!) dependencies.add(question)
    questionDependencies.set(next, dependencies)
    order.push(next)
  }
  const seen = new Set<string>()
  const rules = logic.rules.map((rule, index) => {
    const path = `$.logic.rules[${index}]`
    const position = positions.get(rule.question_id)
    if (position === undefined || seen.has(rule.question_id))
      throw new LogicError('invalid_reference', path, rule.question_id)
    seen.add(rule.question_id)
    if (rule.visible_if == null && rule.required_if == null && rule.validate == null)
      throw new LogicError('invalid_syntax', path, rule.question_id)
    const compileRule = (raw: unknown, field: string, allowSelf: boolean) => {
      if (raw == null) return null
      const fieldPath = `${path}.${field}`
      const expression = compile(raw, fieldPath)
      if (expression.valueType !== 'boolean')
        throw new LogicError('type_mismatch', fieldPath, rule.question_id)
      const dependencies = new Set(expression.dependencies)
      for (const variable of expression.variableDependencies)
        for (const question of questionDependencies.get(variable)!) dependencies.add(question)
      if (
        [...dependencies].some(
          (question) =>
            positions.get(question)! > position ||
            (!allowSelf && positions.get(question) === position),
        )
      )
        throw new LogicError('forward_reference', fieldPath, rule.question_id)
      return expression
    }
    return {
      question_id: rule.question_id,
      visible_if: compileRule(rule.visible_if, 'visible_if', false),
      required_if: compileRule(rule.required_if, 'required_if', false),
      validate: compileRule(rule.validate, 'validate', true),
    }
  })
  return Object.freeze({
    variableOrder: Object.freeze([...order]),
    evaluate(answers: Readonly<Record<string, unknown>>): DocumentLogicValues {
      const values = new Map<string, LogicValue>()
      for (const id of order) values.set(id, variables.get(id)!.evaluate(answers, values))
      const results: DocumentLogicValues['rules'] = {}
      for (const rule of rules)
        results[rule.question_id] = {
          visible_if: rule.visible_if?.evaluate(answers, values) ?? null,
          required_if: rule.required_if?.evaluate(answers, values) ?? null,
          validate: rule.validate?.evaluate(answers, values) ?? null,
        }
      return { variables: Object.fromEntries(values), rules: results }
    },
  })
}

function expressionBytes(expression: unknown): number {
  return new TextEncoder().encode(JSON.stringify(expression)).length
}

/** 整卷复制：先建立全部身份映射，再重写规则；不改变来源文档。 */
export function duplicateSurveyDocument(document: SurveyDocument): SurveyDocument {
  const duplicated = copy(document)
  const questions = new Map<string, string>(),
    options = new Map<string, string>(),
    variables = new Map<string, string>()
  for (const question of duplicated.questions) {
    const next = crypto.randomUUID()
    questions.set(question.id, next)
    question.id = next
    if ('options' in question)
      for (const option of question.options) {
        const next = crypto.randomUUID()
        options.set(option.id, next)
        option.id = next
      }
  }
  if (duplicated.logic) {
    for (const variable of duplicated.logic.variables)
      variables.set(variable.id, crypto.randomUUID())
    for (const variable of duplicated.logic.variables) {
      variable.id = variables.get(variable.id)!
      rewrite(variable.expression, questions, options, variables)
    }
    for (const rule of duplicated.logic.rules) {
      rule.question_id = questions.get(rule.question_id) ?? rule.question_id
      for (const expression of [rule.visible_if, rule.required_if, rule.validate])
        rewrite(expression, questions, options, variables)
    }
  }
  return duplicated
}

function rewrite(
  raw: unknown,
  questions: Map<string, string>,
  options: Map<string, string>,
  variables: Map<string, string>,
) {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return
  const expression = raw as Record<string, unknown>
  for (const [key, ids] of [
    ['question_id', questions],
    ['option_id', options],
    ['variable_id', variables],
  ] as const) {
    const value = expression[key]
    if (typeof value === 'string' && ids.has(value)) expression[key] = ids.get(value)
  }
  if (Array.isArray(expression.option_ids))
    expression.option_ids = expression.option_ids.map((value) => options.get(value) ?? value)
  for (const key of ['expression', 'left', 'right', 'value', 'fallback'])
    rewrite(expression[key], questions, options, variables)
  if (Array.isArray(expression.expressions))
    for (const value of expression.expressions) rewrite(value, questions, options, variables)
}

/** 单题复制保留外部依赖，并将直接引用自身的约束指向新题目。 */
export function duplicateQuestionRule(
  document: SurveyDocument,
  source: Question,
  duplicated: Question,
) {
  const rule = document.logic?.rules.find((rule) => rule.question_id === source.id)
  if (!rule || !document.logic) return
  const copied = copy(rule)
  copied.question_id = duplicated.id
  const questions = new Map([[source.id, duplicated.id]])
  const options = new Map<string, string>()
  if ('options' in source && 'options' in duplicated)
    source.options.forEach((option, index) => {
      const replacement = duplicated.options[index]
      if (replacement) options.set(option.id, replacement.id)
    })
  for (const expression of [copied.visible_if, copied.required_if, copied.validate])
    rewrite(expression, questions, options, new Map())
  document.logic.rules.push(copied)
}
