//! 新题目与复制题目生成独立身份，调整顺序不改变已有身份。
import { copy } from './api'
import type { Question } from './types'
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
      return { ...base, type, max_length: type === 'text' ? 200 : 1000 }
    case 'number':
      return { ...base, type, integer: false, min: null, max: null }
    case 'rating':
      return { ...base, type, min: 1, max: 5, min_label: '很不满意', max_label: '非常满意' }
  }
}
export function duplicateQuestion(question: Question) {
  const duplicated = copy(question)
  duplicated.id = crypto.randomUUID()
  if ('options' in duplicated)
    duplicated.options.forEach((option) => {
      option.id = crypto.randomUUID()
    })
  return duplicated
}
