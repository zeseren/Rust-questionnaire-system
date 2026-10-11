//! 新题目与复制题目生成独立身份，调整顺序不改变已有身份。
import { copy } from './json.ts'
import { renewDocumentIds } from './wasm/index.ts'
import type { Question, SurveyDocument } from './types.ts'
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

/** 整卷复制：身份重写交给 wasm 引擎，题目、选项、计算变量与流程节点的
 * 全部引用同步更新；来源文档不被修改。 */
export function duplicateSurveyDocument(document: SurveyDocument): SurveyDocument {
  return renewDocumentIds(copy(document))
}

/** 单题复制保留外部依赖，并将直接引用自身的约束指向新题目。 */
export function duplicateQuestionRule(
  document: SurveyDocument,
  source: Question,
  duplicated: Question,
) {
  for (const node of document.logic?.flow?.nodes ?? []) {
    if (node.type !== 'page') continue
    const position = node.question_ids.indexOf(source.id)
    if (position >= 0) node.question_ids.splice(position + 1, 0, duplicated.id)
  }
  const rule = document.logic?.rules.find((rule) => rule.question_id === source.id)
  if (!rule || !document.logic) return
  const copied = copy(rule)
  copied.question_id = duplicated.id
  // 单题复制只重写来源题与自身约束：外部依赖必须原样保留，因此这里用动态身份映射。
  const questions = new Map<string, string>()
  questions.set(source.id, duplicated.id)
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

/** 按身份映射改写表达式里的引用；非对象节点原样返回。 */
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
