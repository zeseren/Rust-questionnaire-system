//! 流程编辑器的拓扑工具：插入节点并让新节点接管前驱的实际出口。
//! 只做结构改写，不校验语义；语义由 wasm 引擎在编译/求值时报错并定位。
import type { FlowDefinition, FlowNode } from './types.ts'
import { newId } from './uuid.ts'

/**
 * 在 index 后插入新节点，由新节点接管前驱当前指向的出口，返回新节点身份；无法插入时返回 null。
 *
 * 接管规则（按前驱类型）：
 * - page：接管唯一出口 `next`，前驱改为指向新节点。`next` 为空说明页面尚未接出口，拒绝插入。
 * - branch：只接管兜底出口 `otherwise`，显式分支目标保持原路由不变。`otherwise` 为空时拒绝插入。
 * - end：结束节点没有出口，在其后插入必然产生不可达节点，直接拒绝。
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
  if (!previous || previous.type === 'end') return null
  const target = previous.type === 'page' ? previous.next : previous.otherwise
  if (!target) return null
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
