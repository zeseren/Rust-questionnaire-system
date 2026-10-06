//! 界面保留原始输入，流程只接收正确 JSON 类型；提交数字不能变成超范围裸整数。
import { LosslessNumber } from 'lossless-json'
import { FlowSession } from './flow.ts'
import { LogicError } from './logic.ts'
import { copy } from './json.ts'
import type { Inputs, InputValue } from './answers.ts'
import type { Answer, SurveyDocument } from './types'

export class FlowInputs {
  #document: SurveyDocument
  #session: FlowSession
  #values: Inputs = {}

  constructor(document: SurveyDocument) {
    this.#document = copy(document)
    this.#session = new FlowSession(this.#document)
  }
  snapshot() {
    return this.#session.snapshot()
  }
  values(): Inputs {
    return copy(this.#values)
  }
  change(questionId: string, value: InputValue) {
    const question = this.#document.questions.find((question) => question.id === questionId)
    let typed: unknown = value
    if ((question?.type === 'number' || question?.type === 'rating') && typeof value === 'string') {
      const raw = value.trim().replace(/([eE])(?=\d)/, '$1+')
      if (!raw) typed = null
      else if (raw.length <= 64 && /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/.test(raw))
        typed = new LosslessNumber(raw)
      // 非法数字原文作为错误类型交给引擎，不能转成 missing 或先舍入。
    }
    this.#session.dispatch({ type: 'change', question_id: questionId, value: typed })
    this.#values[questionId] = Array.isArray(value) ? [...value] : value
    for (const id of this.snapshot().cleared) delete this.#values[id]
  }
  next() {
    this.#session.dispatch({ type: 'next' })
  }
  previous() {
    this.#session.dispatch({ type: 'previous' })
  }
  submissionAnswers(): Answer[] {
    const snapshot = this.snapshot()
    if (snapshot.outcome !== 'complete') throw new LogicError('invalid_event', '$.event')
    // 指数形式保证 1e20 等规范化结果在传输时不被 JS 展开成超出 u64 的裸整数。
    return Object.entries(snapshot.answers)
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([question_id, value]) => ({
        question_id,
        value:
          typeof value === 'number'
            ? new LosslessNumber(value.toExponential())
            : Array.isArray(value)
              ? [...value]
              : value,
      }))
  }
}
