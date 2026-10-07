//! 数字输入按原始 JSON 数字发送，最终精度与资格校验由服务器执行。
import { LosslessNumber } from 'lossless-json'
import type { Answer, SurveyDocument } from './types'
import { numberError, numberInput } from './numbers.ts'
import { isBlank, validText } from './value-contract.ts'
export type InputValue = string | string[] | number | null
export type Inputs = Record<string, InputValue>
export function isAnswered(value: InputValue | undefined) {
  return value != null && (Array.isArray(value) ? value.length > 0 : !isBlank(String(value)))
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
      if (!validText(value)) {
        fail('回答不能包含空字符或无效 Unicode 字符。')
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
        const raw = numberInput(String(value))
        const error = numberError(raw)
        if (error) {
          fail(error)
          break
        }
        const numeric = Number(raw)
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
