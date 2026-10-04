//! 规则先编译后执行；三值逻辑、数字边界和错误分类与 Rust 共用语义样例。
import { isLosslessNumber } from 'lossless-json'
import type { Question } from './types'

export type LogicValueType = 'boolean' | 'number' | 'text' | 'choice' | 'choices'
export type LogicValue =
  | { type: 'missing' }
  | { type: 'boolean'; value: boolean }
  | { type: 'number'; value: number }
  | { type: 'text' | 'choice'; value: string }
  | { type: 'choices'; value: string[] }

export type LogicErrorCode =
  | 'invalid_syntax'
  | 'unsupported_version'
  | 'limit_exceeded'
  | 'invalid_reference'
  | 'type_mismatch'
  | 'invalid_number'
  | 'invalid_answer'
  | 'arithmetic_error'

const messages: Record<LogicErrorCode, string> = {
  invalid_syntax: '规则结构或字段无效',
  unsupported_version: '不支持此规则版本',
  limit_exceeded: '规则超出容量限制',
  invalid_reference: '规则引用的题目或选项无效',
  type_mismatch: '规则的操作数类型不匹配',
  invalid_number: '规则数字无效或无法精确表示',
  invalid_answer: '规则上下文包含无效答案',
  arithmetic_error: '规则计算发生除零或数值溢出',
}

export class LogicError extends Error {
  readonly code: LogicErrorCode
  readonly path: string
  readonly question_id: string | null

  constructor(code: LogicErrorCode, path: string, questionId: string | null = null) {
    super(messages[code])
    this.name = 'LogicError'
    this.code = code
    this.path = path
    this.question_id = questionId
  }
}

type Comparison = 'eq' | 'ne' | 'lt' | 'le' | 'gt' | 'ge'
type Arithmetic = 'add' | 'subtract' | 'multiply' | 'divide'
type Binding = { type: LogicValueType; options: Set<string> }
type Node = { type: LogicValueType; path: string; kind: NodeKind }
type NodeKind =
  | { op: 'literal'; value: LogicValue }
  | { op: 'answer' | 'answered'; id: string }
  | { op: 'selection'; id: string; options: string[]; all: boolean }
  | { op: 'not'; expression: Node }
  | { op: 'logical'; expressions: Node[]; all: boolean }
  | { op: 'compare'; comparison: Comparison; left: Node; right: Node }
  | { op: 'coalesce'; value: Node; fallback: Node }
  | { op: 'length'; value: Node }
  | { op: 'arithmetic'; operator: Arithmetic; left: Node; right: Node }

/** 仅由编译入口创建；不会暴露可修改的 AST 或题目绑定。 */
export interface CompiledExpression {
  readonly valueType: LogicValueType
  readonly dependencies: readonly string[]
  /** 输入须由问卷层规范化并排除不适用答案；此处防御性检查类型与选项身份。 */
  evaluate(answers: Readonly<Record<string, unknown>>): LogicValue
}

function fail(code: LogicErrorCode, path: string, id: string | null = null): never {
  throw new LogicError(code, path, id)
}

function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) fail('invalid_syntax', '$')
  return value as Record<string, unknown>
}

function fields(value: Record<string, unknown>, allowed: string[]) {
  if (
    Object.keys(value).length !== allowed.length ||
    allowed.some((key) => !Object.hasOwn(value, key))
  ) {
    fail('invalid_syntax', '$')
  }
}

function string(value: unknown): string {
  if (typeof value !== 'string') fail('invalid_syntax', '$')
  return value
}

function reference(value: unknown, path: string): string {
  const raw = string(value)
  if (
    !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(raw) ||
    raw === '00000000-0000-0000-0000-000000000000'
  )
    fail('invalid_reference', path)
  return raw
}

function validText(value: string) {
  // JS 允许孤立代理项，Rust String 不允许；按同一 Unicode 标量契约拒绝。
  return !/[\0\uD800-\uDFFF]/u.test(value)
}

function numeric(raw: string): number | undefined {
  if (raw.length > 64 || !/^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/.test(raw))
    return undefined
  const normalized = raw.replace(/([eE])(?=\d)/, '$1+')
  if (normalized.length > 64) return undefined
  const value = Number(raw)
  if (!Number.isFinite(value)) return undefined
  if (!/[.eE]/.test(raw)) {
    const integer = BigInt(raw)
    if (integer < -(1n << 63n) || integer > (1n << 64n) - 1n || integer !== BigInt(value))
      return undefined
  }
  return value === 0 ? 0 : value
}

function checkBudget(raw: unknown, depth: number, budget: { count: number }) {
  budget.count++
  if (depth > 16 || budget.count > 256) fail('limit_exceeded', '$.expression')
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return
  const record = raw as Record<string, unknown>
  for (const key of ['expression', 'left', 'right', 'fallback', 'value']) {
    const child = record[key]
    if (child && typeof child === 'object' && !Array.isArray(child))
      checkBudget(child, depth + 1, budget)
  }
  if (Array.isArray(record.expressions)) {
    for (const child of record.expressions) checkBudget(child, depth + 1, budget)
  }
}

/** 版本、字段、类型、常量、引用或资源限制不合法时抛出 LogicError。 */
export function compileExpression(
  raw: unknown,
  questions: readonly Question[],
): CompiledExpression {
  const program = object(raw)
  checkBudget(program.expression, 1, { count: 0 })
  fields(program, ['dsl_version', 'semantics_version', 'expression'])
  for (const version of [program.dsl_version, program.semantics_version]) {
    if (
      typeof version !== 'number' ||
      !Number.isInteger(version) ||
      version < 0 ||
      version > 4294967295
    ) {
      fail('invalid_syntax', '$')
    }
  }
  if (program.dsl_version !== 1 || program.semantics_version !== 1) fail('unsupported_version', '$')
  if (questions.length > 100) fail('limit_exceeded', '$.questions')
  const bindings = new Map<string, Binding>()
  for (const question of questions) {
    const id = reference(question.id, '$.questions')
    if (bindings.has(id)) fail('invalid_reference', '$.questions', id)
    let type: LogicValueType
    let options: Set<string> = new Set()
    switch (question.type) {
      case 'single_choice':
      case 'multiple_choice':
        type = question.type === 'single_choice' ? 'choice' : 'choices'
        options = new Set(question.options.map((option) => reference(option.id, '$.questions')))
        if (options.size !== question.options.length) fail('invalid_reference', '$.questions', id)
        if (options.size > 50) fail('limit_exceeded', '$.questions', id)
        break
      case 'text':
      case 'textarea':
        type = 'text'
        break
      case 'number':
      case 'rating':
        type = 'number'
        break
    }
    bindings.set(id, { type, options })
  }
  const dependencies = new Set<string>()
  const root = compileNode(program.expression, '$.expression', bindings, dependencies)
  const ids = Object.freeze([...dependencies].sort())
  return Object.freeze({
    valueType: root.type,
    dependencies: ids,
    evaluate(answers: Readonly<Record<string, unknown>>) {
      const context = new Map<string, LogicValue>()
      for (const id of ids) {
        const binding = bindings.get(id)
        if (!binding) fail('invalid_reference', '$.answers', id)
        context.set(
          id,
          readAnswer(Object.hasOwn(answers, id) ? answers[id] : undefined, binding, id),
        )
      }
      return evaluate(root, context)
    },
  })
}

function compileNode(
  raw: unknown,
  path: string,
  bindings: Map<string, Binding>,
  dependencies: Set<string>,
): Node {
  const expr = object(raw)
  const op = string(expr.op)
  const child = (key: string) => compileNode(expr[key], `${path}.${key}`, bindings, dependencies)
  const bound = () => {
    const id = reference(expr.question_id, path)
    const binding = bindings.get(id)
    if (!binding) fail('invalid_reference', path, id)
    dependencies.add(id)
    return { id, binding }
  }
  const node = (type: LogicValueType, kind: NodeKind): Node => ({ type, kind, path })
  switch (op) {
    case 'boolean': {
      fields(expr, ['op', 'value'])
      if (typeof expr.value !== 'boolean') fail('invalid_syntax', '$')
      return node('boolean', { op: 'literal', value: { type: 'boolean', value: expr.value } })
    }
    case 'number': {
      fields(expr, ['op', 'value'])
      const value = numeric(string(expr.value))
      if (value === undefined) fail('invalid_number', path)
      return node('number', { op: 'literal', value: { type: 'number', value } })
    }
    case 'text': {
      fields(expr, ['op', 'value'])
      const value = string(expr.value)
      if (!validText(value)) fail('invalid_syntax', path)
      if ([...value].length > 5000) fail('limit_exceeded', path)
      return node('text', { op: 'literal', value: { type: 'text', value } })
    }
    case 'answer':
    case 'answered': {
      fields(expr, ['op', 'question_id'])
      const { id, binding } = bound()
      return node(op === 'answered' ? 'boolean' : binding.type, { op, id })
    }
    case 'selected':
    case 'contains_any':
    case 'contains_all': {
      fields(expr, ['op', 'question_id', op === 'selected' ? 'option_id' : 'option_ids'])
      const { id, binding } = bound()
      if (binding.type !== 'choices' && (op !== 'selected' || binding.type !== 'choice'))
        fail('type_mismatch', path, id)
      const values = op === 'selected' ? [string(expr.option_id)] : expr.option_ids
      if (!Array.isArray(values) || values.some((value) => typeof value !== 'string'))
        fail('invalid_syntax', '$')
      if (!values.length || values.length > 50) fail('invalid_syntax', path, id)
      const options = new Set<string>()
      for (const value of values) {
        if (!binding.options.has(value) || options.has(value)) fail('invalid_reference', path, id)
        options.add(value)
      }
      return node('boolean', {
        op: 'selection',
        id,
        options: [...options].sort(),
        all: op === 'contains_all',
      })
    }
    case 'not': {
      fields(expr, ['op', 'expression'])
      const expression = child('expression')
      if (expression.type !== 'boolean') fail('type_mismatch', path)
      return node('boolean', { op, expression })
    }
    case 'all':
    case 'any': {
      fields(expr, ['op', 'expressions'])
      if (!Array.isArray(expr.expressions)) fail('invalid_syntax', '$')
      if (!expr.expressions.length) fail('invalid_syntax', path)
      const expressions = expr.expressions.map((value, index) => {
        const result = compileNode(value, `${path}.expressions[${index}]`, bindings, dependencies)
        if (result.type !== 'boolean') fail('type_mismatch', path)
        return result
      })
      return node('boolean', { op: 'logical', expressions, all: op === 'all' })
    }
    case 'compare': {
      fields(expr, ['op', 'comparison', 'left', 'right'])
      const comparison = string(expr.comparison)
      if (!['eq', 'ne', 'lt', 'le', 'gt', 'ge'].includes(comparison)) fail('invalid_syntax', '$')
      const left = child('left'),
        right = child('right')
      if (
        left.type !== right.type ||
        !['boolean', 'number', 'text'].includes(left.type) ||
        (!['eq', 'ne'].includes(comparison) && left.type !== 'number')
      )
        fail('type_mismatch', path)
      return node('boolean', { op, comparison: comparison as Comparison, left, right })
    }
    case 'coalesce': {
      fields(expr, ['op', 'value', 'fallback'])
      const value = child('value'),
        fallback = child('fallback')
      if (value.type !== fallback.type) fail('type_mismatch', path)
      return node(value.type, { op, value, fallback })
    }
    case 'length': {
      fields(expr, ['op', 'value'])
      const value = child('value')
      if (value.type !== 'text' && value.type !== 'choices') fail('type_mismatch', path)
      return node('number', { op, value })
    }
    case 'arithmetic': {
      fields(expr, ['op', 'operator', 'left', 'right'])
      const operator = string(expr.operator)
      if (!['add', 'subtract', 'multiply', 'divide'].includes(operator)) fail('invalid_syntax', '$')
      const left = child('left'),
        right = child('right')
      if (left.type !== 'number' || right.type !== 'number') fail('type_mismatch', path)
      return node('number', { op, operator: operator as Arithmetic, left, right })
    }
    default:
      return fail('invalid_syntax', '$')
  }
}

function readAnswer(raw: unknown, binding: Binding, id: string): LogicValue {
  const path = `$.answers.${id}`
  if (raw == null) return { type: 'missing' }
  switch (binding.type) {
    case 'number': {
      if (typeof raw !== 'number' && !isLosslessNumber(raw)) fail('invalid_answer', path, id)
      const value = numeric(raw.toString())
      if (value === undefined) fail('invalid_answer', path, id)
      return { type: 'number', value }
    }
    case 'text':
      if (typeof raw !== 'string' || !validText(raw) || [...raw].length > 5000)
        fail('invalid_answer', path, id)
      return { type: 'text', value: raw }
    case 'choice':
      if (typeof raw !== 'string' || !binding.options.has(raw)) fail('invalid_answer', path, id)
      return { type: 'choice', value: raw }
    case 'choices':
      if (
        !Array.isArray(raw) ||
        raw.length > 50 ||
        raw.some((value) => typeof value !== 'string' || !binding.options.has(value)) ||
        new Set(raw).size !== raw.length
      )
        fail('invalid_answer', path, id)
      return { type: 'choices', value: [...raw].sort() }
    case 'boolean':
      return fail('invalid_answer', path, id)
  }
}

function evaluate(node: Node, context: Map<string, LogicValue>): LogicValue {
  const kind = node.kind
  const missing: LogicValue = { type: 'missing' }
  const boolean = (value: boolean): LogicValue => ({ type: 'boolean', value })
  const number = (value: number): LogicValue => ({ type: 'number', value: value === 0 ? 0 : value })
  switch (kind.op) {
    case 'literal':
      return { ...kind.value }
    case 'answer': {
      const value = context.get(kind.id) ?? missing
      return value.type === 'choices' ? { type: 'choices', value: [...value.value] } : { ...value }
    }
    case 'answered':
      return boolean(context.has(kind.id) && context.get(kind.id)?.type !== 'missing')
    case 'selection': {
      const value = context.get(kind.id) ?? missing
      if (value.type === 'missing') return missing
      if (value.type === 'choice') return boolean(kind.options.includes(value.value))
      if (value.type !== 'choices') fail('type_mismatch', node.path)
      return boolean(
        kind.all
          ? kind.options.every((id) => value.value.includes(id))
          : kind.options.some((id) => value.value.includes(id)),
      )
    }
    case 'not': {
      const value = evaluate(kind.expression, context)
      if (value.type === 'missing') return missing
      if (value.type !== 'boolean') fail('type_mismatch', node.path)
      return boolean(!value.value)
    }
    case 'logical': {
      let unknown = false
      for (const expression of kind.expressions) {
        const value = evaluate(expression, context)
        if (value.type === 'missing') unknown = true
        else if (value.type !== 'boolean') fail('type_mismatch', node.path)
        else if (value.value !== kind.all) return boolean(value.value)
      }
      return unknown ? missing : boolean(kind.all)
    }
    case 'coalesce': {
      const value = evaluate(kind.value, context)
      return value.type === 'missing' ? evaluate(kind.fallback, context) : value
    }
    case 'compare': {
      const left = evaluate(kind.left, context),
        right = evaluate(kind.right, context)
      if (left.type === 'missing' || right.type === 'missing') return missing
      if (left.type !== right.type || left.type === 'choices' || right.type === 'choices')
        fail('type_mismatch', node.path)
      switch (kind.comparison) {
        case 'eq':
          return boolean(left.value === right.value)
        case 'ne':
          return boolean(left.value !== right.value)
        case 'lt':
          return boolean(left.value < right.value)
        case 'le':
          return boolean(left.value <= right.value)
        case 'gt':
          return boolean(left.value > right.value)
        case 'ge':
          return boolean(left.value >= right.value)
      }
    }
    case 'length': {
      const value = evaluate(kind.value, context)
      if (value.type === 'missing') return missing
      if (value.type !== 'text' && value.type !== 'choices') fail('type_mismatch', node.path)
      return number([...value.value].length)
    }
    case 'arithmetic': {
      const left = evaluate(kind.left, context),
        right = evaluate(kind.right, context)
      if (left.type === 'missing' || right.type === 'missing') return missing
      if (left.type !== 'number' || right.type !== 'number') fail('type_mismatch', node.path)
      let result: number
      switch (kind.operator) {
        case 'add':
          result = left.value + right.value
          break
        case 'subtract':
          result = left.value - right.value
          break
        case 'multiply':
          result = left.value * right.value
          break
        case 'divide':
          result = left.value / right.value
          break
      }
      if (!Number.isFinite(result)) fail('arithmetic_error', node.path)
      return number(result)
    }
  }
}
