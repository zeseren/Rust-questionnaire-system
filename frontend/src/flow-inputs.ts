//! 界面保留原始输入，流程只接收正确 JSON 类型。
import { FlowSession } from './wasm/index.ts'
import { LogicError } from './logic.ts'
import { isNumberLiteral, isNumberPrefix, numberInput } from './numbers.ts'
import type { Inputs, InputValue } from './answers.ts'
import type { Answer, FlowEvent, FlowSnapshot, SurveyDocument } from './types.ts'

export class FlowInputs {
  #document: SurveyDocument
  #session: FlowSession
  #snapshot: FlowSnapshot
  #values: Inputs = {}
  /** 只留在界面镜像、引擎尚未见过的数字前缀；这些题的快照错误不代表当前输入。 */
  #unsettled = new Set<string>()

  constructor(document: SurveyDocument) {
    this.#document = document
    this.#session = new FlowSession(this.#document)
    this.#snapshot = this.#session.snapshot()
  }
  // 返回最近一次事件后缓存的只读快照，调用方不得就地修改。
  snapshot() {
    return this.#snapshot
  }
  values(): Inputs {
    // 数组值写入时已克隆，基础类型不可变，消费方只读，浅拷贝即可。
    return { ...this.#values }
  }
  /** 输入仍停留在未结算数字前缀的题目；返回副本，调用方可安全持有。 */
  unsettled(): ReadonlySet<string> {
    return new Set(this.#unsettled)
  }
  change(questionId: string, value: InputValue) {
    this.#apply(questionId, value, false)
  }
  /** 失焦时结算过渡前缀：把界面原文按完整输入再送一次，及时显示校验结果。 */
  commit(questionId: string) {
    const value = this.#values[questionId]
    if (typeof value === 'string') this.#apply(questionId, value, true)
  }
  next() {
    this.#flushTransitions()
    this.#session.dispatch({ type: 'next' })
    this.#snapshot = this.#session.snapshot()
  }
  previous() {
    this.#session.dispatch({ type: 'previous' })
    this.#snapshot = this.#session.snapshot()
  }
  submissionAnswers(): Answer[] {
    const snapshot = this.snapshot()
    // 到达 complete 或 screen_out 都可以提交；两者都会生成回执。
    if (!snapshot.outcome) throw new LogicError('invalid_event', '$.event')
    // 答案统一为原生 JSON 值；数组按值克隆，避免暴露会话快照内部结构。
    return Object.entries(snapshot.answers)
      .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
      .map(([question_id, value]) => ({
        question_id,
        value: Array.isArray(value) ? [...value] : value,
      }))
  }

  /** 当前步是否接受对该题的改答：未到终点、题目可见且属于当前页面。 */
  #editable(questionId: string): boolean {
    if (this.#snapshot.outcome || !this.#snapshot.visible.includes(questionId)) return false
    const nodes = this.#document.logic?.flow?.nodes
    // 未配置流程的隐式单页没有文档节点，当前页即全部可见题目。
    if (!nodes) return true
    const node = nodes.find((node) => node.id === this.#snapshot.current_node)
    return node?.type === 'page' && node.question_ids.includes(questionId)
  }

  #apply(questionId: string, value: InputValue, settled: boolean) {
    const question = this.#document.questions.find((question) => question.id === questionId)
    if (
      !settled &&
      typeof value === 'string' &&
      (question?.type === 'number' || question?.type === 'rating')
    ) {
      // 过渡前缀：只留在界面镜像里，等完整数字或失焦结算时才送引擎。
      if (isNumberPrefix(numberInput(value))) {
        if (!this.#editable(questionId))
          throw new LogicError('invalid_event', '$.event', questionId)
        this.#values[questionId] = value
        this.#unsettled.add(questionId)
        return
      }
    }
    let typed: unknown = value
    if ((question?.type === 'number' || question?.type === 'rating') && typeof value === 'string') {
      const raw = numberInput(value)
      if (!raw) typed = null
      else if (isNumberLiteral(raw)) typed = Number(raw)
      // 非法数字原文作为错误类型交给引擎，不能转成 missing 或先舍入。
    }
    this.#dispatch(questionId, typed, value)
  }

  #dispatch(questionId: string, typed: unknown, value: InputValue) {
    this.#session.dispatch({ type: 'change', question_id: questionId, value: typed })
    this.#snapshot = this.#session.snapshot()
    this.#values[questionId] = Array.isArray(value) ? [...value] : value
    this.#unsettled.delete(questionId)
    for (const id of this.#snapshot.cleared) {
      delete this.#values[id]
      this.#unsettled.delete(id)
    }
  }

  /** 翻页前把仍未结算的数字前缀按完整输入送给引擎。 */
  #flushTransitions() {
    for (const questionId of [...this.#unsettled]) {
      const value = this.#values[questionId]
      if (typeof value === 'string') this.#apply(questionId, value, true)
    }
  }
}

/** 回放结果：`applied` 是成功应用的事件前缀长度，失败或形状无效后不再尝试后续事件。 */
export interface ReplayResult {
  applied: number
  total: number
  /** 已成功应用并收窄后的前缀；失败事件不会进入。 */
  events: FlowEvent[]
}

type ReplayEvent =
  | { type: 'change'; question_id: string; value: InputValue }
  | { type: 'next' }
  | { type: 'previous' }

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && [...value].every((item: unknown) => typeof item === 'string')
}

/** 本地存储可能被任意改写：只把完整输入值收窄为界面镜像接受的类型。 */
function replayValue(value: unknown): InputValue | undefined {
  if (value === null || typeof value === 'string') return value
  if (typeof value === 'number') return Number.isFinite(value) ? value : undefined
  if (isStringArray(value)) return [...value]
  return undefined
}

function replayEvent(raw: unknown, plain: boolean): ReplayEvent | undefined {
  if (typeof raw !== 'object' || raw === null || !('type' in raw)) return undefined
  if (raw.type === 'change') {
    if (!('question_id' in raw) || typeof raw.question_id !== 'string') return undefined
    if (!('value' in raw)) return undefined
    const value = replayValue(raw.value)
    if (value === undefined) return undefined
    return { type: 'change', question_id: raw.question_id, value }
  }
  // 普通问卷没有分步导航，混入的 next/previous 不是可回放的进度。
  if (plain) return undefined
  if (raw.type === 'next') return { type: 'next' }
  if (raw.type === 'previous') return { type: 'previous' }
  return undefined
}

/**
 * 按顺序回放暂存进度，只接受形状有效的事件。
 * 形状无效、不可应用或非法导航立即停止并保留已成功前缀；领域内拒绝的答案仍算已回放。
 */
export function replayEvents(
  form: FlowInputs,
  stored: readonly unknown[],
  plain: boolean,
): ReplayResult {
  const events: FlowEvent[] = []
  for (const raw of stored) {
    const event = replayEvent(raw, plain)
    if (!event) break
    try {
      if (event.type === 'change') form.change(event.question_id, event.value)
      else if (event.type === 'next') form.next()
      else form.previous()
    } catch {
      break
    }
    events.push(event)
  }
  return { applied: events.length, total: stored.length, events }
}
