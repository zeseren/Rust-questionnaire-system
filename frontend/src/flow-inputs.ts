//! 界面保留原始输入，流程只接收正确 JSON 类型。
import { FlowSession } from './wasm/index.ts'
import { LogicError } from './logic.ts'
import { numberError, numberInput } from './numbers.ts'
import type { Inputs, InputValue } from './answers.ts'
import type { Answer, FlowSnapshot, SurveyDocument } from './types.ts'

/** 完整有限数字的文本形式；`12.` 与 `1e` 在补全前都不算完整。 */
const COMPLETE_NUMBER = /^-?(?:\d+(?:\.\d+)?|\.\d+)(?:[eE][+-]?\d+)?$/
/** 可继续补全为数字的过渡前缀：单独负号、`12.`、`1e`、`1e-` 等。 */
const NUMBER_PREFIX = /^(?:-|-?\.\d*|-?\d+(?:\.\d*)?|-?(?:\d+(?:\.\d*)?|\.\d+)[eE][+-]?\d*)$/

/** 输入尚未完成但仍是合法数字前缀；此时不把原文交给引擎判为无效答案。 */
export function transitionalNumber(raw: string): boolean {
  return !COMPLETE_NUMBER.test(raw) && NUMBER_PREFIX.test(raw)
}

export class FlowInputs {
  #document: SurveyDocument
  #session: FlowSession
  #snapshot: FlowSnapshot
  #values: Inputs = {}

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

  #apply(questionId: string, value: InputValue, settled: boolean) {
    const question = this.#document.questions.find((question) => question.id === questionId)
    if (
      !settled &&
      typeof value === 'string' &&
      (question?.type === 'number' || question?.type === 'rating')
    ) {
      const raw = numberInput(value)
      // 过渡前缀：只留在界面镜像里，等完整数字或失焦结算时才送引擎。
      if (raw && transitionalNumber(raw)) {
        this.#values[questionId] = value
        return
      }
    }
    let typed: unknown = value
    if ((question?.type === 'number' || question?.type === 'rating') && typeof value === 'string') {
      const raw = numberInput(value)
      if (!raw) typed = null
      else if (!numberError(raw)) typed = Number(raw)
      // 非法数字原文作为错误类型交给引擎，不能转成 missing 或先舍入。
    }
    this.#dispatch(questionId, typed, value)
  }

  #dispatch(questionId: string, typed: unknown, value: InputValue) {
    this.#session.dispatch({ type: 'change', question_id: questionId, value: typed })
    this.#snapshot = this.#session.snapshot()
    this.#values[questionId] = Array.isArray(value) ? [...value] : value
    for (const id of this.#snapshot.cleared) delete this.#values[id]
  }

  /** 翻页前把仍处于过渡状态的数字原文结算为完整输入。 */
  #flushTransitions() {
    for (const [questionId, value] of Object.entries(this.#values)) {
      if (typeof value !== 'string') continue
      const question = this.#document.questions.find((item) => item.id === questionId)
      if (question?.type !== 'number' && question?.type !== 'rating') continue
      const raw = numberInput(value)
      if (raw && transitionalNumber(raw)) this.#apply(questionId, value, true)
    }
  }
}
