//! 简洁条件只接受可无损表达的节点，复杂表达式继续交给高级编辑器。
import type { Question } from './types'

export type ConditionOperator =
  | 'answered'
  | 'not_answered'
  | 'selected'
  | 'not_selected'
  | 'eq'
  | 'ne'
  | 'gt'
  | 'ge'
  | 'lt'
  | 'le'
export interface SimpleCondition {
  questionId: string
  operator: ConditionOperator
  value: string
  valueType?: 'number' | 'text'
}
export interface ConditionGroup {
  mode: 'always' | 'never' | 'all' | 'any'
  conditions: SimpleCondition[]
}
export const comparisonNames = {
  eq: '等于',
  ne: '不等于',
  gt: '大于',
  ge: '大于或等于',
  lt: '小于',
  le: '小于或等于',
} as const

export function changeConditionOperator(
  condition: SimpleCondition,
  operator: ConditionOperator,
  question?: Question,
): SimpleCondition {
  const questionId = condition.questionId
  if (operator === 'answered' || operator === 'not_answered')
    return { questionId, operator, value: '' }
  if (operator === 'selected' || operator === 'not_selected') {
    const value =
      condition.operator === 'selected' || condition.operator === 'not_selected'
        ? condition.value
        : question && 'options' in question
          ? (question.options[0]?.id ?? '')
          : ''
    return { questionId, operator, value }
  }
  const valueType = question?.type === 'text' || question?.type === 'textarea' ? 'text' : 'number'
  const compatible =
    Object.hasOwn(comparisonNames, condition.operator) &&
    (condition.valueType ?? 'number') === valueType
  return {
    questionId,
    operator,
    valueType,
    value: compatible ? condition.value : valueType === 'number' ? '0' : '',
  }
}

function record(raw: unknown): Record<string, unknown> {
  return raw && typeof raw === 'object' && !Array.isArray(raw)
    ? (raw as Record<string, unknown>)
    : {}
}
function keys(node: Record<string, unknown>, names: string[]) {
  return Object.keys(node).length === names.length && names.every((key) => Object.hasOwn(node, key))
}
function readCondition(raw: unknown): SimpleCondition | null {
  const node = record(raw)
  if (node.op === 'not' && keys(node, ['op', 'expression'])) {
    const child = readCondition(node.expression)
    if (child?.operator === 'answered') return { ...child, operator: 'not_answered' }
    if (child?.operator === 'selected') return { ...child, operator: 'not_selected' }
  }
  if (
    node.op === 'answered' &&
    keys(node, ['op', 'question_id']) &&
    typeof node.question_id === 'string'
  )
    return { questionId: node.question_id, operator: 'answered', value: '' }
  if (
    node.op === 'selected' &&
    keys(node, ['op', 'question_id', 'option_id']) &&
    typeof node.question_id === 'string' &&
    typeof node.option_id === 'string'
  )
    return { questionId: node.question_id, operator: 'selected', value: node.option_id }
  const left = record(node.left),
    right = record(node.right)
  if (
    node.op === 'compare' &&
    keys(node, ['op', 'comparison', 'left', 'right']) &&
    typeof node.comparison === 'string' &&
    Object.hasOwn(comparisonNames, node.comparison) &&
    left.op === 'answer' &&
    keys(left, ['op', 'question_id']) &&
    typeof left.question_id === 'string' &&
    (right.op === 'number' || right.op === 'text') &&
    keys(right, ['op', 'value']) &&
    typeof right.value === 'string'
  )
    return {
      questionId: left.question_id,
      operator: node.comparison as ConditionOperator,
      value: right.value,
      valueType: right.op,
    }
  return null
}

export function readConditionGroup(raw: unknown): ConditionGroup | null {
  const node = record(raw)
  if (node.op === 'boolean' && keys(node, ['op', 'value']) && typeof node.value === 'boolean')
    return { mode: node.value ? 'always' : 'never', conditions: [] }
  if (
    (node.op === 'all' || node.op === 'any') &&
    keys(node, ['op', 'expressions']) &&
    Array.isArray(node.expressions)
  ) {
    const conditions = node.expressions.map(readCondition)
    if (conditions.every((condition) => condition !== null)) return { mode: node.op, conditions }
    return null
  }
  const condition = readCondition(node)
  return condition ? { mode: 'all', conditions: [condition] } : null
}

export function conditionExpression(condition: SimpleCondition): unknown {
  const question_id = condition.questionId
  switch (condition.operator) {
    case 'answered':
      return { op: 'answered', question_id }
    case 'not_answered':
      return { op: 'not', expression: { op: 'answered', question_id } }
    case 'selected':
      return { op: 'selected', question_id, option_id: condition.value }
    case 'not_selected':
      return { op: 'not', expression: { op: 'selected', question_id, option_id: condition.value } }
    default:
      return {
        op: 'compare',
        comparison: condition.operator,
        left: { op: 'answer', question_id },
        right: { op: condition.valueType ?? 'number', value: condition.value },
      }
  }
}

export function groupExpression(group: ConditionGroup): unknown {
  if (group.mode === 'always' || group.mode === 'never')
    return { op: 'boolean', value: group.mode === 'always' }
  const expressions = group.conditions.map(conditionExpression)
  return { op: group.mode, expressions }
}

export function conditionOperators(
  question?: Question,
): { value: ConditionOperator; label: string }[] {
  const options: { value: ConditionOperator; label: string }[] = [
    { value: 'answered', label: '已回答' },
    { value: 'not_answered', label: '未回答' },
  ]
  if (question && 'options' in question)
    options.push({ value: 'selected', label: '选中了' }, { value: 'not_selected', label: '未选中' })
  else if (question)
    for (const [value, label] of Object.entries(comparisonNames)) {
      if (
        (question.type === 'text' || question.type === 'textarea') &&
        value !== 'eq' &&
        value !== 'ne'
      )
        continue
      options.push({ value: value as ConditionOperator, label })
    }
  return options
}
