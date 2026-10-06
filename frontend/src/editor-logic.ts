//! 编辑期定位结构问题；实际保存仍以同一领域编译器的结果为准。
import { compileDocumentLogic } from './document-logic.ts'
import { LogicError } from './logic.ts'
import type { DocumentLogic } from './document-logic.ts'
import type { Question } from './types'

export interface LogicDiagnostic {
  message: string
  target: string
  path: string
}
export function logicDiagnostic(
  logic: DocumentLogic,
  questions: Question[],
): LogicDiagnostic | null {
  const flow = logic.flow
  if (flow) {
    const incoming = new Set<string>()
    const membership: string[] = []
    for (const [index, node] of flow.nodes.entries()) {
      const path = `$.logic.flow.nodes[${index}]`
      const fail = (message: string) => ({ message, target: `logic-node-${node.id}`, path })
      if (index === 0 && (node.type !== 'page' || node.id !== flow.entry))
        return fail('入口必须是第一个页面。')
      if (node.type === 'page') {
        if (!node.question_ids.length) return fail('页面至少需要一道题，请分配题目或删除空页。')
        membership.push(...node.question_ids)
        if (node.question_ids.some((id) => !questions.some((q) => q.id === id)))
          return fail('页面包含已删除题目，请移除失效归属。')
      }
      const targets =
        node.type === 'page'
          ? [node.next]
          : node.type === 'branch'
            ? [...node.branches.map((b) => b.target), node.otherwise]
            : []
      if (targets.some((id) => !flow.nodes.slice(index + 1).some((n) => n.id === id)))
        return fail('出口必须指向后续存在的节点，请修正跳转。')
      targets.forEach((id) => incoming.add(id))
      if (index > 0 && !incoming.has(node.id))
        return fail('没有前置节点指向这里，请配置进入路径或删除节点。')
    }
    if (
      membership.length !== questions.length ||
      membership.some((id, i) => id !== questions[i]?.id)
    )
      return {
        message: '每题须归属一个页面，页面顺序必须与题目顺序一致。请调整题目归属或题目顺序。',
        target: 'logic-membership',
        path: '$.logic.flow',
      }
  }
  try {
    compileDocumentLogic(logic, questions)
    return null
  } catch (reason) {
    if (!(reason instanceof LogicError)) throw reason
    const variable = /variables\[(\d+)\]/.exec(reason.path)
    const rule = /rules\[(\d+)\]/.exec(reason.path)
    const node = /nodes\[(\d+)\]/.exec(reason.path)
    const target = variable
      ? `logic-variable-${logic.variables[Number(variable[1])]?.id}`
      : rule
        ? `logic-rule-${Number(rule[1])}`
        : node
          ? `logic-node-${flow?.nodes[Number(node[1])]?.id}`
          : reason.question_id
            ? `question-${reason.question_id}`
            : 'logic-editor'
    return { message: reason.message, target, path: reason.path }
  }
}
