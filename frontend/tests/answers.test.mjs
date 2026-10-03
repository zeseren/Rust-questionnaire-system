//! 预览与正式填写共用的数字校验必须拒绝失真整数，并保留后端允许的大整数。
import assert from 'node:assert/strict'
import test from 'node:test'
import { buildAnswers } from '../src/answers.ts'

const questionId = '00000000-0000-4000-8000-000000000001'

function document(integer) {
  return {
    title: '数字题回归',
    description: '',
    purpose: '核对预览和提交的整数精度规则',
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

test('预览和填写均拒绝会被舍入的裸整数', () => {
  for (const integer of [true, false]) {
    for (const raw of ['9007199254740993', '-9007199254740993', '18446744073709551615']) {
      const result = buildAnswers(document(integer), { [questionId]: raw })
      assert.ok(result.errors[questionId], `应定位到失真整数所在题目：${raw}`)
      assert.equal(result.answers.length, 0)
    }
  }
})

test('裸整数不能越过后端的有符号下界和无符号上界', () => {
  for (const raw of ['-9223372036854775809', '18446744073709551616']) {
    const result = buildAnswers(document(true), { [questionId]: raw })
    assert.ok(result.errors[questionId], `应拒绝超出整数范围的输入：${raw}`)
    assert.equal(result.answers.length, 0)
  }
})

test('可精确表示的大整数和既有指数写法仍可提交', () => {
  for (const raw of [
    '0',
    '-0',
    '9007199254740992',
    '9007199254740994',
    '-9223372036854775808',
    '18446744073709549568',
    '1e+20',
  ]) {
    const result = buildAnswers(document(true), { [questionId]: raw })
    assert.deepEqual(result.errors, {}, `不应误拒绝后端支持的输入：${raw}`)
    assert.equal(result.answers.length, 1)
    assert.equal(result.answers[0].value.toString(), raw)
  }
})
