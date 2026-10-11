//! 流程只能指向后续节点；合流处的分支独有答案必须显式处理缺失。
import { LogicError } from './logic.ts'
import { isConfigurationId } from './value-contract.ts'
import type { CompiledExpression } from './logic.ts'
import type { CompiledDocumentLogic, DocumentLogic } from './document-logic.ts'
import type { Question } from './types'

export type FlowOutcome = 'complete' | 'screen_out'
export interface FlowBranch {
  when: unknown
  target: string
}
export type FlowNode =
  | { type: 'page'; id: string; question_ids: string[]; next: string }
  | { type: 'branch'; id: string; branches: FlowBranch[]; otherwise: string }
  | { type: 'end'; id: string; outcome: FlowOutcome }
export interface FlowDefinition {
  entry: string
  nodes: FlowNode[]
}
export type CompiledFlowNode =
  | {
      readonly type: 'page'
      readonly id: string
      readonly questions: readonly string[]
      readonly next: number
    }
  | {
      readonly type: 'branch'
      readonly guards: readonly { readonly when: CompiledExpression; readonly target: number }[]
      readonly otherwise: number
    }
  | { readonly type: 'end'; readonly id: string; readonly outcome: FlowOutcome }
export interface CompiledFlow {
  readonly nodes: readonly CompiledFlowNode[]
}

function invalid(): never {
  throw new LogicError('invalid_flow', '$.logic.flow')
}
function fields(raw: unknown, names: string[]): Record<string, unknown> {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return invalid()
  const value = raw as Record<string, unknown>
  if (
    Object.keys(value).length !== names.length ||
    names.some((name) => !Object.hasOwn(value, name))
  )
    return invalid()
  return value
}
function uuid(raw: unknown): string {
  if (!isConfigurationId(raw)) return invalid()
  return raw
}

/** 只解析结构，AST 随文档统一进行容量、引用和类型检查。 */
export function parseFlow(raw: unknown): FlowDefinition {
  const flow = fields(raw, ['entry', 'nodes'])
  if (!Array.isArray(flow.nodes)) return invalid()
  if (flow.nodes.length > 256) throw new LogicError('limit_exceeded', '$.logic.flow')
  const nodes = flow.nodes.map((raw): FlowNode => {
    if (!raw || typeof raw !== 'object') return invalid()
    const type = (raw as Record<string, unknown>).type
    if (type === 'page') {
      const node = fields(raw, ['type', 'id', 'question_ids', 'next'])
      if (!Array.isArray(node.question_ids)) return invalid()
      return {
        type,
        id: uuid(node.id),
        question_ids: node.question_ids.map(uuid),
        next: uuid(node.next),
      }
    }
    if (type === 'branch') {
      const node = fields(raw, ['type', 'id', 'branches', 'otherwise'])
      if (!Array.isArray(node.branches) || node.branches.length < 1 || node.branches.length > 16)
        return invalid()
      return {
        type,
        id: uuid(node.id),
        otherwise: uuid(node.otherwise),
        branches: node.branches.map((raw) => {
          const branch = fields(raw, ['when', 'target'])
          return { when: branch.when, target: uuid(branch.target) }
        }),
      }
    }
    if (type === 'end') {
      const node = fields(raw, ['type', 'id', 'outcome'])
      if (node.outcome !== 'complete' && node.outcome !== 'screen_out') return invalid()
      return { type, id: uuid(node.id), outcome: node.outcome }
    }
    return invalid()
  })
  return { entry: uuid(flow.entry), nodes }
}

export function compileFlow(
  flow: FlowDefinition,
  definition: DocumentLogic,
  logic: CompiledDocumentLogic,
  questions: readonly Question[],
  compile: (raw: unknown, path: string) => CompiledExpression,
): CompiledFlow {
  if (flow.nodes[0]?.type !== 'page' || flow.nodes[0].id !== flow.entry) return invalid()
  const occupied = new Set([
    ...questions.flatMap((question) => [
      question.id,
      ...('options' in question ? question.options.map((option) => option.id) : []),
    ]),
    ...definition.variables.map((variable) => variable.id),
  ])
  const indices = new Map<string, number>()
  flow.nodes.forEach((node, index) => {
    if (occupied.has(node.id)) return invalid()
    occupied.add(node.id)
    indices.set(node.id, index)
  })
  const pages = new Map<string, number>(),
    flattened: string[] = []
  const predecessors = flow.nodes.map(() => [] as number[])
  flow.nodes.forEach((node, index) => {
    if (node.type === 'page') {
      if (!node.question_ids.length) return invalid()
      for (const id of node.question_ids) {
        flattened.push(id)
        pages.set(id, index)
      }
    }
    const targets =
      node.type === 'page'
        ? [node.next]
        : node.type === 'branch'
          ? [...node.branches.map((branch) => branch.target), node.otherwise]
          : []
    for (const target of targets) {
      const next = indices.get(target)
      if (next === undefined || next <= index) return invalid()
      predecessors[next]!.push(index)
    }
  })
  if (
    flattened.length !== questions.length ||
    flattened.some((id, index) => id !== questions[index]!.id)
  )
    return invalid()
  const dominators: Set<number>[] = []
  predecessors.forEach((parents, index) => {
    if (index > 0 && !parents.length) return invalid()
    const common = new Set(index === 0 ? [] : dominators[parents[0]!]!)
    for (const parent of parents)
      for (const node of common) if (!dominators[parent]!.has(node)) common.delete(node)
    common.add(index)
    dominators.push(common)
  })
  const variables = new Map(
    definition.variables.map((variable) => [variable.id, variable.expression]),
  )
  const reads = new Map<string, Set<string>>()
  for (const id of logic.variableOrder) reads.set(id, requiredReads(variables.get(id), reads))
  const available = (raw: unknown, node: number, path: string) => {
    if ([...requiredReads(raw, reads)].some((id) => !dominators[node]!.has(pages.get(id)!)))
      throw new LogicError('unavailable_dependency', path)
  }
  definition.rules.forEach((rule, index) => {
    for (const field of ['visible_if', 'required_if', 'validate'] as const)
      if (rule[field] != null)
        available(rule[field], pages.get(rule.question_id)!, `$.logic.rules[${index}].${field}`)
  })
  const nodes = flow.nodes.map((node, index): CompiledFlowNode => {
    if (node.type === 'page')
      return Object.freeze({
        type: 'page',
        id: node.id,
        questions: Object.freeze([...node.question_ids]),
        next: indices.get(node.next)!,
      })
    if (node.type === 'end') return Object.freeze({ ...node })
    const guards = node.branches.map((branch, position) => {
      const path = `$.logic.flow.nodes[${index}].branches[${position}].when`
      const when = compile(branch.when, path)
      if (when.valueType !== 'boolean') throw new LogicError('type_mismatch', path)
      const raw = branch.when as Record<string, unknown>
      if (raw.op === 'boolean' && (raw.value === false || position + 1 < node.branches.length))
        throw new LogicError('invalid_flow', path)
      if ([...logic.dependencies(when)].some((id) => pages.get(id)! >= index))
        throw new LogicError('forward_reference', path)
      available(branch.when, index, path)
      return Object.freeze({ when, target: indices.get(branch.target)! })
    })
    return Object.freeze({
      type: 'branch',
      guards: Object.freeze(guards),
      otherwise: indices.get(node.otherwise)!,
    })
  })
  return Object.freeze({ nodes: Object.freeze(nodes) })
}

// 只遍历已经编译成功的 AST；缓存变量摘要，避免重复展开形成指数工作量。
function requiredReads(raw: unknown, variables: Map<string, Set<string>>): Set<string> {
  if (!raw || typeof raw !== 'object') return new Set()
  const value = raw as Record<string, unknown>
  switch (value.op) {
    case 'answer':
    case 'selected':
    case 'contains_any':
    case 'contains_all':
      return new Set([value.question_id as string])
    case 'variable':
      return new Set(variables.get(value.variable_id as string))
    case 'answered':
      return new Set()
    case 'coalesce':
      return requiredReads(value.fallback, variables)
    case 'all':
    case 'any': {
      const reads = new Set<string>(),
        guarded = new Set<string>()
      for (const expression of value.expressions as unknown[]) {
        for (const id of requiredReads(expression, variables)) if (!guarded.has(id)) reads.add(id)
        for (const id of presenceGuards(expression, value.op === 'all')) guarded.add(id)
      }
      return reads
    }
    default:
      return new Set(
        ['expression', 'left', 'right', 'value'].flatMap((key) => [
          ...requiredReads(value[key], variables),
        ]),
      )
  }
}
function presenceGuards(raw: unknown, truth: boolean): Set<string> {
  if (!raw || typeof raw !== 'object') return new Set()
  const value = raw as Record<string, unknown>
  if (value.op === 'answered' && truth) return new Set([value.question_id as string])
  if (value.op === 'not') return presenceGuards(value.expression, !truth)
  if ((value.op === 'all' && truth) || (value.op === 'any' && !truth))
    return new Set(
      (value.expressions as unknown[]).flatMap((expression) => [
        ...presenceGuards(expression, truth),
      ]),
    )
  return new Set()
}

/** 在 index 后插入新节点，回接前驱所有指向原后继的出口，返回新节点身份。 */
export function insertFlowNode(flow: FlowDefinition, index: number, type: FlowNode['type']): string {
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
    for (const branch of previous.branches)
      if (branch.target === target) branch.target = id
  }
  flow.nodes.splice(index + 1, 0, node)
  return id
}
