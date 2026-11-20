//! 答案输入类型与不可信提交载荷的形状校验；题型规则由 wasm 引擎执行。
import type { Answer, AnswerValue, Submission } from './types.ts'
import { isBlank } from './value-contract.ts'
export type InputValue = string | string[] | number | null
export type Inputs = Record<string, InputValue>
export function isAnswered(value: InputValue | undefined) {
  if (value == null) return false
  if (Array.isArray(value)) return value.length > 0
  return typeof value === 'string' ? !isBlank(value) : true
}
function isAnswerValue(value: unknown): value is AnswerValue {
  if (value === null || typeof value === 'string' || typeof value === 'number') return true
  return Array.isArray(value) && value.every((item: unknown) => typeof item === 'string')
}
function isAnswer(value: unknown): value is Answer {
  if (typeof value !== 'object' || value === null) return false
  if (!('question_id' in value) || typeof value.question_id !== 'string') return false
  return 'value' in value && isAnswerValue(value.value)
}
/** 校验从本地存储等不可信来源读回的提交载荷；字段缺失或类型不符一律不认。 */
export function isSubmission(value: unknown): value is Submission {
  if (typeof value !== 'object' || value === null) return false
  if (!('version_id' in value) || typeof value.version_id !== 'string') return false
  if (!('idempotency_key' in value) || typeof value.idempotency_key !== 'string') return false
  return 'answers' in value && Array.isArray(value.answers) && value.answers.every(isAnswer)
}
