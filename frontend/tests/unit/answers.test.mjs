//! 答案输入判定与不可信提交载荷的形状校验。
import assert from 'node:assert/strict'
import test from 'node:test'
import { isAnswered, isSubmission } from '../../src/answers.ts'

test('isAnswered 按空白、数组长度与原生类型判断', () => {
  for (const value of [null, undefined, '', '  ', []])
    assert.equal(isAnswered(value), false, JSON.stringify(value))
  for (const value of [0, -0, ['a']]) assert.equal(isAnswered(value), true, JSON.stringify(value))
})

test('isSubmission 只接受字段完整且答案值类型合法的提交载荷', () => {
  const submission = {
    version_id: 'v1',
    idempotency_key: 'k1',
    answers: [
      { question_id: 'a', value: '文本' },
      { question_id: 'b', value: 3 },
      { question_id: 'c', value: ['x', 'y'] },
      { question_id: 'd', value: null },
    ],
  }
  assert.equal(isSubmission(submission), true)
  assert.equal(isSubmission({ ...submission, answers: [] }), true)

  const rejected = [
    null,
    'text',
    [],
    { ...submission, version_id: 1 },
    { version_id: 'v1', answers: [] },
    { ...submission, answers: {} },
    { ...submission, answers: [null] },
    { ...submission, answers: [{ question_id: 'a' }] },
    { ...submission, answers: [{ question_id: 1, value: 'x' }] },
    { ...submission, answers: [{ question_id: 'a', value: true }] },
    { ...submission, answers: [{ question_id: 'a', value: [1] }] },
    { ...submission, answers: [{ question_id: 'a', value: { nested: 1 } }] },
  ]
  for (const value of rejected) assert.equal(isSubmission(value), false, JSON.stringify(value))
})
