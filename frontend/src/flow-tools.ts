//! 流程编辑器的拓扑工具：插入节点、调整题目的页面归属。
//! 只做结构改写，不校验语义；语义由 wasm 引擎在编译/求值时报错并定位。
import type { FlowDefinition, FlowNode, Question } from './types.ts'
import { newId } from './uuid.ts'

type PageNode = Extract<FlowNode, { type: 'page' }>

/** 在该节点后插入时由新节点接管的出口；结束节点与尚未接出口的节点返回 null。 */
export function insertionExit(node: FlowNode): string | null {
  if (node.type === 'end') return null
  return (node.type === 'page' ? node.next : node.otherwise) || null
}

/**
 * 题目可以归属的页面：各页题目依次拼接后必须等于全局题目顺序，
 * 所以目标页不能早于任何排在它前面的题所在的页，也不能晚于任何排在它后面的题所在的页。
 * 未归属的题目同样按这条规则落位；题目不在问卷里时返回空集合。
 */
export function assignablePageIds(
  flow: FlowDefinition,
  questions: readonly Question[],
  questionId: string,
): Set<string> {
  const order = new Map(questions.map((question, position) => [question.id, position]))
  const position = order.get(questionId)
  const legal = new Set<string>()
  if (position === undefined) return legal
  const pages = flow.nodes.filter((node): node is PageNode => node.type === 'page')
  let lastEarlier = -1
  let firstLater = pages.length
  pages.forEach((page, index) => {
    for (const id of page.question_ids) {
      const other = order.get(id)
      if (other === undefined || other === position) continue
      if (other < position) lastEarlier = index
      else firstLater = Math.min(firstLater, index)
    }
  })
  pages.forEach((page, index) => {
    if (lastEarlier <= index && index <= firstLater) legal.add(page.id)
  })
  return legal
}

/** 把题目移到目标页并保持页内全局顺序；目标页不合法时不改动流程并返回 false。 */
export function assignQuestion(
  flow: FlowDefinition,
  questions: readonly Question[],
  questionId: string,
  pageId: string,
): boolean {
  if (!assignablePageIds(flow, questions, questionId).has(pageId)) return false
  const order = new Map(questions.map((question, position) => [question.id, position]))
  for (const page of flow.nodes) {
    if (page.type !== 'page') continue
    const kept = page.question_ids.filter((id) => id !== questionId)
    if (page.id === pageId) {
      kept.push(questionId)
      kept.sort((a, b) => (order.get(a) ?? -1) - (order.get(b) ?? -1))
    } else if (kept.length === page.question_ids.length) continue
    page.question_ids = kept
  }
  return true
}

/**
 * 在 index 后插入新节点，由新节点接管前驱的出口（页面的 `next`、分支的兜底 `otherwise`），
 * 显式分支目标保持原路由；前驱没有可接管的出口时不改动流程并返回 null。
 *
 * 新节点固定落在 index + 1：既排在前驱之后，又把原出口目标推到自己之后，
 * 满足 logic-core 的前向拓扑约束（所有跳转必须指向数组中更靠后的节点）。
 */
export function insertFlowNode(
  flow: FlowDefinition,
  index: number,
  type: FlowNode['type'],
): string | null {
  const previous = flow.nodes[index]
  const target = previous && insertionExit(previous)
  if (!previous || previous.type === 'end' || !target) return null
  const id = newId()
  const node: FlowNode =
    type === 'page'
      ? { type, id, question_ids: [], next: target }
      : type === 'branch'
        ? {
            type,
            id,
            branches: [{ when: { op: 'boolean', value: true }, target }],
            otherwise: target,
          }
        : { type, id, outcome: 'complete' }
  if (previous.type === 'page') previous.next = id
  else previous.otherwise = id
  flow.nodes.splice(index + 1, 0, node)
  return id
}
