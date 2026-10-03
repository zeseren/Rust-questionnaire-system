//! 数字输入按原始 JSON 数字发送，最终精度与资格校验由服务器执行。
import { LosslessNumber } from 'lossless-json'
import type { Answer, SurveyDocument } from './types'
// 与后端 domain::MAX_NUMBER_JSON_BYTES 一致，按传输表示限制，不能先舍入数字。
const MAX_NUMBER_JSON_BYTES = 64
// 与后端 finite_number 的裸整数范围一致：i64 最小值至 u64 最大值。
const MIN_JSON_INTEGER = -(1n << 63n)
const MAX_JSON_INTEGER = (1n << 64n) - 1n
export type InputValue = string | string[] | number | null
export type Inputs = Record<string, InputValue>
export function isAnswered(value: InputValue | undefined) {
  return value != null && (Array.isArray(value) ? value.length > 0 : String(value).trim() !== '')
}
export function buildAnswers(document: SurveyDocument, values: Inputs) {
  const answers: Answer[] = []
  const errors: Record<string, string> = {}
  for (const question of document.questions) {
    const value = values[question.id]
    const fail = (text: string) => {
      errors[question.id] = text
    }
    if ((question.type === 'text' || question.type === 'textarea') && typeof value === 'string') {
      if (value.includes('\0')) {
        fail('回答不能包含空字符。')
        continue
      }
      if ([...value].length > question.max_length) {
        fail(`最多填写 ${question.max_length} 个字符。`)
        continue
      }
      if (question.type === 'text' && /[\n\r\v\f\u0085\u2028\u2029]/.test(value)) {
        fail('单行文本不能包含换行。')
        continue
      }
    }
    if (!isAnswered(value)) {
      if (question.required) fail('请回答这道必填题。')
      continue
    }
    switch (question.type) {
      case 'single_choice':
        if (typeof value !== 'string' || !question.options.some((option) => option.id === value))
          fail('请选择一个有效选项。')
        else answers.push({ question_id: question.id, value })
        break
      case 'multiple_choice':
        if (
          !Array.isArray(value) ||
          value.length < question.min_choices ||
          value.length > question.max_choices
        )
          fail(`请选择 ${question.min_choices}–${question.max_choices} 项。`)
        else answers.push({ question_id: question.id, value: [...value] })
        break
      case 'text':
      case 'textarea':
        answers.push({ question_id: question.id, value: String(value) })
        break
      case 'number': {
        // 与 serde_json 一致补齐正指数符号，保持数值和原始整数精度。
        const raw = String(value)
          .trim()
          .replace(/([eE])(?=\d)/, '$1+')
        const numeric = Number(raw)
        if (
          !/^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/.test(raw) ||
          !Number.isFinite(numeric)
        ) {
          fail('请输入有限数字，例如 12、0.5 或 1e3。')
          break
        }
        if (raw.length > MAX_NUMBER_JSON_BYTES) {
          fail(`数字最多 ${MAX_NUMBER_JSON_BYTES} 个字符（含小数点和指数符号）。`)
          break
        }
        // 只对裸整数核对原值；部分超过 Number 安全整数范围的值仍能被精确表示。
        if (!/[.eE]/.test(raw)) {
          const integer = BigInt(raw)
          if (integer < MIN_JSON_INTEGER || integer > MAX_JSON_INTEGER) {
            fail('整数超出支持的数值范围。')
            break
          }
          if (integer !== BigInt(numeric)) {
            fail('整数无法精确表示，请调整数值。')
            break
          }
        }
        if (question.integer && !Number.isInteger(numeric)) fail('此题只接受整数。')
        else if (
          (question.min != null && numeric < question.min) ||
          (question.max != null && numeric > question.max)
        )
          fail('数值不在允许范围内。')
        else answers.push({ question_id: question.id, value: new LosslessNumber(raw) })
        break
      }
      case 'rating': {
        const numeric = Number(value)
        if (!Number.isInteger(numeric) || numeric < question.min || numeric > question.max)
          fail(`请填写 ${question.min}–${question.max} 之间的整数评分。`)
        else answers.push({ question_id: question.id, value: numeric })
      }
    }
  }
  return { answers, errors }
}
