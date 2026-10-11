//! 流程编辑器的拓扑工具：插入节点并回接所有指向原后继的出口。
//! 只做结构改写，不校验语义；语义由 wasm 引擎在编译/求值时报错并定位。
import type { FlowDefinition, FlowNode } from './types.ts'

/** 在 index 后插入新节点，回接前驱所有指向原后继的出口，返回新节点身份。 */
export function insertFlowNode(
  flow: FlowDefinition,
  index: number,
  type: FlowNode['type'],
): string {
  const id = crypto.randomUUID()
  const target = flow.nodes[index + 1]?.id ?? ''
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
  const previous = flow.nodes[index]
  if (previous?.type === 'page' && previous.next === target) previous.next = id
  if (previous?.type === 'branch') {
    if (previous.otherwise === target) previous.otherwise = id
    for (const branch of previous.branches) if (branch.target === target) branch.target = id
  }
  flow.nodes.splice(index + 1, 0, node)
  return id
}
