//! `src/wasm/index.ts` 包装层契约：信封成功值解码、错误信封还原为领域错误类，
//! 以及有状态 `FlowSession` 的失败注入。语义本身由 `logic-core` 测试与契约夹具覆盖。
import assert from 'node:assert/strict'
import test from 'node:test'
import { LogicError, LogicTextError } from '../../src/logic.ts'
import { FlowSession, formatLogicText, parseLogicText } from '../../src/wasm/index.ts'

const Q_NAME = '00000000-0000-4000-8000-000000000001'
const Q_SCORE = '00000000-0000-4000-8000-000000000002'
const UNKNOWN_QUESTION = '00000000-0000-4000-8000-000000000999'

const questions = [
  { id: Q_NAME, title: '姓名', required: true, type: 'text', max_length: 200 },
  {
    id: Q_SCORE,
    title: '评分',
    required: false,
    type: 'number',
    integer: false,
    min: null,
    max: null,
  },
]

const sessionDocument = {
  title: '包装层契约',
  description: '',
  purpose: '验证 wasm 信封还原',
  thank_you: '感谢参与',
  questions: [{ id: Q_NAME, title: '姓名', required: true, type: 'text', max_length: 200 }],
}

test('parseLogicText 语法错误还原为带行列号的 LogicTextError', () => {
  assert.throws(
    () => parseLogicText('dsl1 semantics 1;', []),
    (error) => {
      assert.ok(error instanceof LogicTextError)
      assert.equal(error.code, 'invalid_syntax')
      assert.equal(error.path, '$')
      assert.equal(error.question_id, null)
      assert.equal(error.line, 1)
      assert.equal(error.column, 1)
      return true
    },
  )
})

test('formatLogicText 失败信封还原为 LogicError 并保留规则路径', () => {
  const logic = {
    dsl_version: 1,
    semantics_version: 1,
    variables: [],
    rules: [{ question_id: UNKNOWN_QUESTION, visible_if: { op: 'boolean', value: true } }],
  }

  assert.throws(
    () => formatLogicText(logic, questions),
    (error) => {
      assert.ok(error instanceof LogicError)
      assert.equal(error instanceof LogicTextError, false)
      assert.equal(error.code, 'invalid_reference')
      assert.equal(error.path, '$.logic.rules[0]')
      assert.equal(error.question_id, UNKNOWN_QUESTION)
      return true
    },
  )
})

test('FlowSession 构造失败还原为领域错误并保留重复的题目身份', () => {
  const duplicated = {
    ...sessionDocument,
    questions: [{ ...sessionDocument.questions[0] }, { ...sessionDocument.questions[0] }],
  }

  assert.throws(
    () => new FlowSession(duplicated),
    (error) => {
      assert.ok(error instanceof LogicError)
      assert.equal(error.code, 'invalid_flow')
      assert.equal(error.path, '$')
      assert.equal(error.question_id, Q_NAME)
      return true
    },
  )
})

test('FlowSession 快照、改答与非法回退遵循契约', () => {
  const session = new FlowSession(sessionDocument)

  const initial = session.snapshot()
  assert.deepEqual(initial.answers, {})
  assert.ok(initial.visible.includes(Q_NAME))

  session.dispatch({ type: 'change', question_id: Q_NAME, value: '张三' })
  const changed = session.snapshot()
  assert.equal(changed.answers[Q_NAME], '张三')

  // 首页不存在上一页，事件失败时不得改变会话状态。
  assert.throws(
    () => session.dispatch({ type: 'previous' }),
    (error) => {
      assert.ok(error instanceof LogicError)
      assert.equal(error.code, 'invalid_event')
      assert.equal(error.path, '$.event')
      return true
    },
  )
  assert.deepEqual(session.snapshot().answers, changed.answers)
})
