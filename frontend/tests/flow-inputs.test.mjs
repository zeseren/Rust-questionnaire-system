//! 防止界面原文与流程状态脱节，以及规范化数字在传输时被展开成非法裸整数。
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import { FlowInputs } from '../src/flow-inputs.ts'
import { copy, copyPayload, decode, encode } from '../src/json.ts'
import { LogicError } from '../src/logic.ts'
import { FlowSession } from '../src/wasm/index.ts'
const fixtures = JSON.parse(
  readFileSync(new URL('../../fixtures/survey-logic/flows.json', import.meta.url), 'utf8'),
)

test('界面改答清除旧分支原文且数值提交保持指数格式', () => {
  const document = copy(fixtures.documents.diamond)
  const [choice, number, text, merge] = document.questions
  const [a, b] = choice.options
  const form = new FlowInputs(document)
  form.change(choice.id, a.id)
  form.next()
  form.change(number.id, '9007199254740993')
  assert.equal(form.snapshot().errors[number.id], 'invalid_answer')
  assert.throws(
    () => form.next(),
    (error) => error instanceof LogicError && error.code === 'invalid_answer',
  )
  form.previous()
  form.change(choice.id, b.id)
  assert.equal(form.values()[number.id], undefined)
  assert.equal(form.snapshot().errors[number.id], undefined)
  form.next()
  form.change(text.id, '乙路径')
  form.previous()
  form.change(choice.id, a.id)
  form.next()
  assert.equal(form.values()[number.id], undefined)
  assert.equal(form.values()[text.id], undefined)
  form.change(number.id, '1e20')
  form.next()
  assert.ok(form.snapshot().required.includes(merge.id))
  form.change(merge.id, '汇合')
  assert.throws(
    () => form.submissionAnswers(),
    (error) => error instanceof LogicError && error.code === 'invalid_event',
    '到达终点前不可提交',
  )
  form.next()
  const json = encode(copyPayload(form.submissionAnswers()))
  assert.ok(json.includes('1e+20'))
  const response = new FlowSession(document).evaluateAnswers(decode(json))
  assert.equal(response.answers[number.id], 1e20)
  assert.equal(response.question_states[text.id], 'skipped')
})
