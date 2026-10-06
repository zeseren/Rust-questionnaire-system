//! 改答原子重算有效路径；历史只由翻页事件推进，失效答案不会在回退时复活。
import { isLosslessNumber } from 'lossless-json'
import { compileDocumentLogic } from './document-logic.ts'
import type { CompiledDocumentLogic } from './document-logic.ts'
import type { CompiledFlow, CompiledFlowNode, FlowOutcome } from './flow-compiler.ts'
import { LogicError, numeric } from './logic.ts'
import type { CompiledExpression, LogicErrorCode, LogicValue } from './logic.ts'
import { copy } from './json.ts'
import type { Question, SurveyDocument } from './types'

export type FlowEvent =
  | { type: 'change'; question_id: string; value: unknown }
  | { type: 'next' }
  | { type: 'previous' }
type Answer = string | string[] | number
interface Projection {
  path: number[]
  visible: string[]
  answers: Record<string, Answer>
  errors: Record<string, LogicErrorCode>
  route_error: LogicErrorCode | null
}
export interface FlowSnapshot extends Omit<Projection, 'path'> {
  current_node: string
  outcome: FlowOutcome | null
  history: string[]
  path: string[]
  cleared: string[]
}

export type AnswerState = 'answered' | 'unanswered' | 'skipped'
export interface EvaluatedResponse {
  answers: Record<string, Answer>
  question_states: Record<string, AnswerState>
  path: string[]
}

/** 会话持有文档副本，外部不能修改答案、编译结果或可信历史。 */
export class FlowSession {
  #document: SurveyDocument
  #logic: CompiledDocumentLogic
  #flow: CompiledFlow
  #inputs: Record<string, unknown> = {}
  #history = [0]
  #projection: Projection
  #cleared: string[] = []

  constructor(document: SurveyDocument) {
    this.#document = copy(document)
    this.#logic = compileDocumentLogic(
      this.#document.logic ?? {
        dsl_version: 1,
        semantics_version: 1,
        variables: [],
        rules: [],
      },
      this.#document.questions,
    )
    this.#flow = this.#logic.flow ?? {
      nodes: [
        {
          type: 'page',
          id: '00000000-0000-0000-0000-000000000000',
          questions: this.#document.questions.map((question) => question.id),
          next: 1,
        },
        { type: 'end', id: '00000000-0000-0000-0000-000000000001', outcome: 'complete' },
      ],
    }
    this.#projection = this.#project(this.#inputs)
  }

  #current(): number {
    return this.#history.at(-1)!
  }
  #id(index: number): string {
    const node = this.#flow.nodes[index]!
    if (node.type === 'branch') throw new LogicError('invalid_event', '$.event')
    return node.id
  }

  /** 返回独立快照，改动快照不会影响会话。 */
  snapshot(): FlowSnapshot {
    const node = this.#flow.nodes[this.#current()]!
    return structuredClone({
      ...this.#projection,
      current_node: this.#id(this.#current()),
      outcome: node.type === 'end' ? node.outcome : null,
      history: this.#history.map((index) => this.#id(index)),
      path: this.#projection.path.map((index) => this.#id(index)),
      cleared: this.#cleared,
    })
  }

  /** 从最终输入重建路径，不读取或修改当前会话的历史与输入；到达 complete 不代表已提交。 */
  evaluateAnswers(answers: readonly { question_id: string; value: unknown }[]): EvaluatedResponse {
    const inputs: Record<string, unknown> = {}
    for (const answer of answers) {
      if (!this.#document.questions.some((question) => question.id === answer.question_id))
        throw new LogicError('invalid_reference', '$.answers', answer.question_id)
      if (Object.hasOwn(inputs, answer.question_id))
        throw new LogicError('invalid_answer', '$.answers', answer.question_id)
      inputs[answer.question_id] = answer.value
    }
    const result = this.#project(inputs)
    for (const question of this.#document.questions) {
      const code = result.errors[question.id]
      if (code) throw new LogicError(code, '$.answers', question.id)
    }
    if (result.route_error) throw new LogicError(result.route_error, '$.route')
    for (const id of Object.keys(inputs).sort())
      if (!result.visible.includes(id)) throw new LogicError('inapplicable_answer', '$.answers', id)
    const final = this.#flow.nodes[result.path.at(-1)!]
    if (final?.type !== 'end') throw new LogicError('unknown_condition', '$.route')
    if (final.outcome === 'screen_out') throw new LogicError('screened_out', '$.route')
    const question_states: Record<string, AnswerState> = {}
    for (const question of this.#document.questions)
      question_states[question.id] = !result.visible.includes(question.id)
        ? 'skipped'
        : Object.hasOwn(result.answers, question.id)
          ? 'answered'
          : 'unanswered'
    return {
      answers: result.answers,
      question_states,
      path: result.path.map((index) => this.#id(index)),
    }
  }

  /** 事件失败不改状态；无效答案保留为输入错误，不进入下游计算。 */
  dispatch(event: FlowEvent): void {
    const invalid = (): never => {
      throw new LogicError('invalid_event', '$.event')
    }
    const node = this.#flow.nodes[this.#current()]!
    switch (event.type) {
      case 'change': {
        if (
          node.type !== 'page' ||
          !node.questions.includes(event.question_id) ||
          !this.#projection.visible.includes(event.question_id)
        )
          return invalid()
        // 原始数字对象保留精度；只通过 JSON 值传递输入，界面字符串由适配层转换。
        const value = typeof event.value === 'number' ? event.value : copy(event.value)
        const inputs = { ...this.#inputs, [event.question_id]: value }
        const projection = this.#project(inputs)
        const cleared = Object.keys(inputs)
          .filter((id) => !projection.visible.includes(id))
          .sort()
        for (const id of cleared) delete inputs[id]
        this.#history = this.#history.filter((index) => projection.path.includes(index))
        this.#inputs = inputs
        this.#projection = projection
        this.#cleared = cleared
        return
      }
      case 'previous':
        if (this.#history.length <= 1) return invalid()
        this.#history.pop()
        this.#cleared = []
        return
      case 'next': {
        if (node.type !== 'page') return invalid()
        for (const id of node.questions) {
          const error = this.#projection.errors[id]
          if (error) throw new LogicError(error, '$.answers', id)
        }
        const position = this.#projection.path.indexOf(this.#current())
        if (position < 0) return invalid()
        const next = this.#projection.path.slice(position + 1).find((index) => {
          const node = this.#flow.nodes[index]!
          return (
            node.type === 'end' ||
            (node.type === 'page' &&
              node.questions.some((id) => this.#projection.visible.includes(id)))
          )
        })
        if (next === undefined)
          throw new LogicError(this.#projection.route_error ?? 'unknown_condition', '$.route')
        this.#history.push(next)
        this.#cleared = []
        return
      }
      default:
        return invalid()
    }
  }

  #evaluate(
    expression: CompiledExpression | null | undefined,
    answers: Record<string, Answer>,
    fallback: boolean,
  ): LogicValue {
    return expression
      ? this.#logic.evaluateExpression(expression, answers)
      : { type: 'boolean', value: fallback }
  }

  #project(inputs: Record<string, unknown>): Projection {
    const result: Projection = { path: [], visible: [], answers: {}, errors: {}, route_error: null }
    let current = 0
    // 编译器已拒绝回边，循环步数不超过节点总数。
    while (true) {
      const node: CompiledFlowNode = this.#flow.nodes[current]!
      if (node.type === 'page') {
        result.path.push(current)
        for (const id of node.questions) {
          const question = this.#document.questions.find((question) => question.id === id)!
          const rule = this.#logic.rule(id)
          try {
            const visible = this.#evaluate(rule?.visible_if, result.answers, true)
            if (visible.type !== 'boolean' || !visible.value) continue
          } catch (error) {
            if (!(error instanceof LogicError)) throw error
            result.visible.push(id)
            result.errors[id] = error.code
            continue
          }
          result.visible.push(id)
          let value: Answer | null
          try {
            value = normalizeAnswer(question, Object.hasOwn(inputs, id) ? inputs[id] : null)
          } catch (error) {
            if (!(error instanceof LogicError)) throw error
            result.errors[id] = error.code
            continue
          }
          try {
            const required = question.required
              ? { type: 'boolean', value: true }
              : this.#evaluate(rule?.required_if, result.answers, false)
            if (required.type === 'missing') result.errors[id] = 'unknown_condition'
            else if ('value' in required && required.value === true && value === null)
              result.errors[id] = 'required_answer'
          } catch (error) {
            if (!(error instanceof LogicError)) throw error
            result.errors[id] = error.code
          }
          if (value !== null) {
            result.answers[id] = value
            try {
              const valid = this.#evaluate(rule?.validate, result.answers, true)
              if (valid.type === 'missing') result.errors[id] = 'unknown_condition'
              else if (valid.type !== 'boolean' || !valid.value)
                result.errors[id] = 'constraint_failed'
            } catch (error) {
              if (!(error instanceof LogicError)) throw error
              result.errors[id] = error.code
            }
            if (result.errors[id]) delete result.answers[id]
          }
        }
        current = node.next
      } else if (node.type === 'branch') {
        let next = node.otherwise
        for (const guard of node.guards) {
          try {
            const value = this.#logic.evaluateExpression(guard.when, result.answers)
            if (value.type !== 'boolean') {
              result.route_error = 'unknown_condition'
              return result
            }
            if (value.value) {
              next = guard.target
              break
            }
          } catch (error) {
            if (!(error instanceof LogicError)) throw error
            result.route_error = error.code
            return result
          }
        }
        current = next
      } else {
        result.path.push(current)
        return result
      }
    }
  }
}

function normalizeAnswer(question: Question, raw: unknown): Answer | null {
  const invalid = (): never => {
    throw new LogicError('invalid_answer', '$.answers', question.id)
  }
  if (raw === null) return null
  const option = (raw: unknown): string => {
    if (typeof raw !== 'string') return invalid()
    // 与 Rust UUID 解析一致接受常见表示，输出统一的小写连字符形式。
    let text = raw.startsWith('urn:uuid:') ? raw.slice(9) : raw
    if (text.startsWith('{') && text.endsWith('}')) text = text.slice(1, -1)
    if (
      !/^(?:[0-9a-f]{32}|[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})$/i.test(text)
    )
      return invalid()
    text = text.replaceAll('-', '').toLowerCase()
    const id = `${text.slice(0, 8)}-${text.slice(8, 12)}-${text.slice(12, 16)}-${text.slice(16, 20)}-${text.slice(20)}`
    if (!('options' in question) || !question.options.some((option) => option.id === id))
      return invalid()
    return id
  }
  switch (question.type) {
    case 'single_choice':
      return option(raw)
    case 'multiple_choice': {
      if (!Array.isArray(raw)) return invalid()
      if (raw.length === 0) return null
      if (raw.length < question.min_choices || raw.length > question.max_choices) return invalid()
      const selected = raw.map(option).sort()
      if (new Set(selected).size !== selected.length) return invalid()
      return selected
    }
    case 'text':
    case 'textarea': {
      if (
        typeof raw !== 'string' ||
        /[\0\uD800-\uDFFF]/u.test(raw) ||
        [...raw].length > question.max_length
      )
        return invalid()
      if (question.type === 'text' && /[\n\r\v\f\u0085\u2028\u2029]/u.test(raw)) return invalid()
      // Rust str::trim 使用 Unicode White_Space；JS trim 对 U+0085、U+FEFF 的定义不同。
      return /^\p{White_Space}*$/u.test(raw) ? null : raw
    }
    case 'number':
    case 'rating': {
      const value =
        typeof raw === 'number'
          ? Number.isFinite(raw)
            ? raw
            : undefined
          : isLosslessNumber(raw)
            ? numeric(raw.value)
            : undefined
      if (value === undefined) return invalid()
      if ((question.type === 'rating' || question.integer) && !Number.isInteger(value))
        return invalid()
      if (
        (question.min !== null && value < question.min) ||
        (question.max !== null && value > question.max)
      )
        return invalid()
      return value === 0 ? 0 : value
    }
  }
}
