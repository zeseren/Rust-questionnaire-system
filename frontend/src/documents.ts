//! 新题目与复制题目生成独立身份，调整顺序不改变已有身份。
import { copy } from './json.ts'
import { duplicateQuestionRule } from './document-logic.ts'
import type { Question, SurveyDocument } from './types'
// 与后端文本答案容量一致；新题默认使用上限，发布者可以调低。
export const TEXT_ANSWER_LIMITS = { text: 200, textarea: 1000 } as const
export function newQuestion(type: Question['type']): Question {
  const base = { id: crypto.randomUUID(), title: '', required: true }
  const options = () => [
    { id: crypto.randomUUID(), label: '选项 1' },
    { id: crypto.randomUUID(), label: '选项 2' },
  ]
  switch (type) {
    case 'single_choice':
      return { ...base, type, options: options() }
    case 'multiple_choice':
      return { ...base, type, options: options(), min_choices: 1, max_choices: 2 }
    case 'text':
    case 'textarea':
      return { ...base, type, max_length: TEXT_ANSWER_LIMITS[type] }
    case 'number':
      return { ...base, type, integer: false, min: null, max: null }
    case 'rating':
      return { ...base, type, min: 1, max: 5, min_label: '很不满意', max_label: '非常满意' }
  }
}
export function duplicateQuestion(question: Question, document: SurveyDocument) {
  const duplicated = copy(question)
  duplicated.id = crypto.randomUUID()
  if ('options' in duplicated)
    duplicated.options.forEach((option) => {
      option.id = crypto.randomUUID()
    })
  duplicateQuestionRule(document, question, duplicated)
  return duplicated
}
