//! 界面保留原始输入，流程只接收正确 JSON 类型。
import { FlowSession } from './wasm/index.ts'
import { LogicError } from './logic.ts'
import { copy } from './json.ts'
import { numberError, numberInput } from './numbers.ts'
import type { Inputs, InputValue } from './answers.ts'
import type { Answer, FlowSnapshot, SurveyDocument } from './types.ts'

export class FlowInputs {
  #document: SurveyDocument
  #session: FlowSession
  #snapshot: FlowSnapshot
  #values: Inputs = {}

  constructor(document: SurveyDocument) {
    this.#document = copy(document)
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
    const question = this.#document.questions.find((question) => question.id === questionId)
    let typed: unknown = value
    if ((question?.type === 'number' || question?.type === 'rating') && typeof value === 'string') {
      const raw = numberInput(value)
      if (!raw) typed = null
      else if (!numberError(raw)) typed = Number(raw)
      // 非法数字原文作为错误类型交给引擎，不能转成 missing 或先舍入。
    }
    this.#session.dispatch({ type: 'change', question_id: questionId, value: typed })
    this.#snapshot = this.#session.snapshot()
    this.#values[questionId] = Array.isArray(value) ? [...value] : value
    for (const id of this.#snapshot.cleared) delete this.#values[id]
  }
  next() {
    this.#session.dispatch({ type: 'next' })
    this.#snapshot = this.#session.snapshot()
  }
  previous() {
    this.#session.dispatch({ type: 'previous' })
    this.#snapshot = this.#session.snapshot()
  }
  submissionAnswers(): Answer[] {
    const snapshot = this.snapshot()
    if (snapshot.outcome !== 'complete') throw new LogicError('invalid_event', '$.event')
    // 答案统一为原生 JSON 值；数组按值克隆，避免暴露会话快照内部结构。
    return Object.entries(snapshot.answers)
      .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
      .map(([question_id, value]) => ({
        question_id,
        value: Array.isArray(value) ? [...value] : value,
      }))
  }
}
