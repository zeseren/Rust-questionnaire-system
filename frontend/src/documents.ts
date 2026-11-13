//! 新题目与复制题目生成独立身份，调整顺序不改变已有身份。
import { copy } from './json.ts'
import { newId } from './uuid.ts'
import { renewDocumentIds } from './wasm/index.ts'
import type { FlowNode, Question, SurveyDocument } from './types.ts'
type PageNode = Extract<FlowNode, { type: 'page' }>
// 与后端文本答案容量一致；新题默认使用上限，发布者可以调低。
export const TEXT_ANSWER_LIMITS = { text: 200, textarea: 1000 } as const
/** 按题型取 Question 的具体分支。Question 是基础字段与联合的相交类型，
 * 直接 Extract 不会对联合分发，故先经 infer 分发再按 type 收窄。 */
type QuestionOf<T extends Question['type']> = Question extends infer Q
  ? Q extends { type: infer K }
    ? T extends K
      ? Q
      : never
    : never
  : never
export function newQuestion<T extends Question['type']>(type: T): QuestionOf<T> {
  const base = { id: newId(), title: '', required: true }
  const options = () => [
    { id: newId(), label: '选项 1' },
    { id: newId(), label: '选项 2' },
  ]
  let question: Question
  switch (type) {
    case 'single_choice':
      question = { ...base, type, options: options() }
      break
    case 'multiple_choice':
      question = { ...base, type, options: options(), min_choices: 1, max_choices: 2 }
      break
    case 'text':
    case 'textarea':
      question = {
        ...base,
        type,
        max_length: TEXT_ANSWER_LIMITS[type === 'text' ? 'text' : 'textarea'],
      }
      break
    case 'number':
      question = { ...base, type, integer: false, min: null, max: null }
      break
    case 'rating':
      question = { ...base, type, min: 1, max: 5, min_label: '很不满意', max_label: '非常满意' }
      break
    default:
      throw new Error('未知题型')
  }
  // 运行时分派已按 T 穷尽，但泛型无法自动映射到对应分支，故此处断言一次。
  return question as QuestionOf<T>
}
export function duplicateQuestion(question: Question, document: SurveyDocument) {
  const duplicated = copy(question)
  duplicated.id = newId()
  if ('options' in duplicated)
    duplicated.options.forEach((option) => {
      option.id = newId()
    })
  duplicateQuestionRule(document, question, duplicated)
  return duplicated
}

/** 整卷复制：身份重写交给 wasm 引擎，题目、选项、计算变量与流程节点的
 * 全部引用同步更新；来源文档不被修改。 */
export function duplicateSurveyDocument(document: SurveyDocument): SurveyDocument {
  // renewDocumentIds 只读取入参并返回新文档，无需先深拷贝。
  return renewDocumentIds(document)
}

/** 将流程页面中的题目归属同步为 `document.questions` 的全局顺序。
 * Rust 流程校验要求所有 `page.question_ids` 依次拼接后严格等于题目 ID 序列，
 * 因此这里先清理失效与重复归属，再把未归属题追加到末页，最后按各页原有槽位
 * 数量重新切分题目列表。这样上移、下移或删除都不会破坏页面覆盖关系。 */
export function syncFlowOrder(document: SurveyDocument): void {
  const flow = document.logic?.flow
  if (!flow) return
  const pages: PageNode[] = []
  for (const node of flow.nodes) if (node.type === 'page') pages.push(node)
  if (!pages.length) return
  const order = new Map(document.questions.map((question, index) => [question.id, index]))
  const assigned = new Set<string>()
  for (const page of pages)
    page.question_ids = page.question_ids.filter((id) => {
      if (!order.has(id) || assigned.has(id)) return false
      assigned.add(id)
      return true
    })
  const last = pages[pages.length - 1]
  if (!last) return
  for (const question of document.questions)
    if (!assigned.has(question.id)) last.question_ids.push(question.id)

  const counts = pages.map((page) => page.question_ids.length)
  const ids = document.questions.map((question) => question.id)
  // 题目数量足够时每页至少保留一题；删除末题后留下的空页会被 Rust 流程校验拒绝。
  if (ids.length >= pages.length)
    for (let index = 0; index < counts.length; index++) {
      if (counts[index] !== 0) continue
      const donor = counts.findIndex((count, position) => position !== index && count > 1)
      if (donor < 0) continue
      counts[donor] = (counts[donor] ?? 0) - 1
      counts[index] = 1
    }
  let cursor = 0
  pages.forEach((page, index) => {
    const count = counts[index] ?? 0
    page.question_ids = ids.slice(cursor, cursor + count)
    cursor += count
  })
  // 身份重复等非法输入可能让槽位少于题目数；余量补到末页以保证拼接恒等。
  if (cursor < ids.length) last.question_ids.push(...ids.slice(cursor))
}

/** 上移/下移题目并同步流程顺序；越界时保持原状。 */
export function moveQuestion(document: SurveyDocument, index: number, direction: number): void {
  const questions = document.questions
  const next = index + direction
  if (index < 0 || index >= questions.length || next < 0 || next >= questions.length) return
  const [question] = questions.splice(index, 1)
  if (!question) return
  questions.splice(next, 0, question)
  syncFlowOrder(document)
}

/** 删除题目并清理直接归属：该题的规则组与流程页面中的题目归属。
 * 表达式中的间接引用交给编译诊断，不在此改写。 */
export function removeQuestion(document: SurveyDocument, questionId: string) {
  const index = document.questions.findIndex((question) => question.id === questionId)
  if (index >= 0) document.questions.splice(index, 1)
  const logic = document.logic
  if (!logic) return
  logic.rules = logic.rules.filter((entry) => entry.question_id !== questionId)
  for (const node of logic.flow?.nodes ?? []) {
    if (node.type !== 'page') continue
    node.question_ids = node.question_ids.filter((id) => id !== questionId)
  }
  syncFlowOrder(document)
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
  const logic = document.logic
  const rule = logic?.rules.find((entry) => entry.question_id === source.id)
  if (rule && logic) {
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
    logic.rules.push(copied)
  }
  // duplicateQuestion 的调用方在返回后才把新题插入 questions；仅在新题已就位时同步，
  // 否则同步会把刚插入页面的身份当作失效引用过滤掉。
  if (document.questions.some((question) => question.id === duplicated.id)) syncFlowOrder(document)
}

function isRecord(raw: unknown): raw is Record<string, unknown> {
  return typeof raw === 'object' && raw !== null && !Array.isArray(raw)
}

/** 按身份映射改写表达式里的引用；非对象节点原样返回。 */
function rewrite(
  raw: unknown,
  questions: Map<string, string>,
  options: Map<string, string>,
  variables: Map<string, string>,
) {
  if (!isRecord(raw)) return
  const expression = raw
  for (const [key, ids] of [
    ['question_id', questions],
    ['option_id', options],
    ['variable_id', variables],
  ] as const) {
    const value = expression[key]
    if (typeof value === 'string' && ids.has(value)) expression[key] = ids.get(value)
  }
  if (Array.isArray(expression.option_ids)) {
    const items: readonly unknown[] = expression.option_ids
    expression.option_ids = items.map((value) =>
      typeof value === 'string' ? (options.get(value) ?? value) : value,
    )
  }
  for (const [key, value] of Object.entries(expression)) {
    if (key === 'question_id' || key === 'option_id' || key === 'variable_id') continue
    if (key === 'option_ids') continue
    if (Array.isArray(value)) {
      const items: readonly unknown[] = value
      for (const item of items) rewrite(item, questions, options, variables)
    } else if (isRecord(value)) rewrite(value, questions, options, variables)
  }
}
