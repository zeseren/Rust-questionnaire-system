//! 预览与正式填写共用的数字校验只接受有限数字，并按题型约束整数。
import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'
import { buildAnswers, isAnswered } from '../src/answers.ts'
import { FlowInputs } from '../src/flow-inputs.ts'

test('共享空白样例保持普通填写与流程的进度必填和省略语义一致', () => {
  const fixtures = JSON.parse(
    readFileSync(
      new URL('../../fixtures/survey-logic/input-contracts.json', import.meta.url),
      'utf8',
    ),
  )
  for (const item of fixtures.text_cases) {
    assert.equal(isAnswered(item.value), item.answered, item.name)
    for (const required of [false, true]) {
      const doc = document(false)
      doc.questions = [
        { id: questionId, title: '多行文本', type: 'textarea', required, max_length: 1000 },
      ]
      const result = buildAnswers(doc, { [questionId]: item.value })
      const form = new FlowInputs(doc)
      form.change(questionId, item.value)
      assert.equal(Boolean(result.errors[questionId]), required && !item.answered, item.name)
      assert.equal(
        Boolean(form.snapshot().errors[questionId]),
        required && !item.answered,
        item.name,
      )
      assert.equal(Object.hasOwn(form.snapshot().answers, questionId), item.answered, item.name)
      assert.equal(result.answers.length, Number(item.answered), item.name)
      if (item.answered) {
        assert.equal(result.answers[0].value, item.value, item.name)
        assert.equal(form.snapshot().answers[questionId], item.value, item.name)
      }
    }
  }
})

const questionId = '00000000-0000-4000-8000-000000000001'

function document(integer) {
  return {
    title: '数字题回归',
    description: '',
    purpose: '核对预览和提交的数字约束',
    thank_you: '',
    questions: [
      {
        id: questionId,
        title: '数字',
        required: true,
        type: 'number',
        integer,
        min: null,
        max: null,
      },
    ],
  }
}

test('数字输入拒绝非有限值、非法写法与非整数', () => {
  for (const raw of ['1e400', 'abc', '1.']) {
    const result = buildAnswers(document(false), { [questionId]: raw })
    assert.ok(result.errors[questionId], `应定位到非法数字所在题目：${raw}`)
    assert.equal(result.answers.length, 0)
  }
  const fractional = buildAnswers(document(true), { [questionId]: '1.5' })
  assert.ok(fractional.errors[questionId], '整数题应拒绝小数')
  assert.equal(fractional.answers.length, 0)
})

test('有限数字与指数写法均可提交', () => {
  for (const raw of ['0', '-0', '42', '1e+20', '1e20']) {
    const result = buildAnswers(document(true), { [questionId]: raw })
    assert.deepEqual(result.errors, {}, `不应误拒绝有限数字：${raw}`)
    assert.equal(result.answers.length, 1)
    assert.equal(result.answers[0].value, Number(raw))
  }
})
