//! 本地答案构建：六种题型的全部接受与拒绝分支。
import assert from 'node:assert/strict'
import test from 'node:test'
import { buildAnswers, isAnswered } from '../../src/answers.ts'
import {
  multipleChoiceQuestion,
  numberQuestion,
  ratingQuestion,
  singleChoiceQuestion,
  surveyDocument,
  textQuestion,
  textareaQuestion,
} from '../helpers/documents.mjs'

const numbersMessage = '请输入有限数字，例如 12、0.5 或 1e3。'
const requiredMessage = '请回答这道必填题。'

test('isAnswered 按空白、数组长度与原生类型判断', () => {
  for (const value of [null, undefined, '', '  ', []])
    assert.equal(isAnswered(value), false, JSON.stringify(value))
  for (const value of [0, -0, ['a']]) assert.equal(isAnswered(value), true, JSON.stringify(value))
})

test('必填未填报错，可选未填与未知题目键都被忽略', () => {
  const required = textQuestion()
  const optional = textQuestion({ id: '00000000-0000-4000-8000-0000000000f1', required: false })
  const number = numberQuestion()
  const rating = ratingQuestion()
  const document = surveyDocument([required, optional, number, rating])
  const result = buildAnswers(document, { unknown: '无关输入' })
  assert.deepEqual(result.answers, [])
  assert.deepEqual(result.errors, {
    [required.id]: requiredMessage,
    [number.id]: requiredMessage,
    [rating.id]: requiredMessage,
  })
})

test('答案按问卷题目顺序输出而不按输入键顺序', () => {
  const number = numberQuestion({ required: false })
  const choice = singleChoiceQuestion({ required: false })
  const document = surveyDocument([number, choice])
  const result = buildAnswers(document, {
    [choice.id]: choice.options[0].id,
    [number.id]: '2',
  })
  assert.deepEqual(result.errors, {})
  assert.deepEqual(result.answers, [
    { question_id: number.id, value: 2 },
    { question_id: choice.id, value: choice.options[0].id },
  ])
})

test('单选只接受属于本题的字符串选项身份', () => {
  const question = singleChoiceQuestion({ required: false })
  const document = surveyDocument([question])
  assert.deepEqual(buildAnswers(document, { [question.id]: question.options[1].id }), {
    answers: [{ question_id: question.id, value: question.options[1].id }],
    errors: {},
  })
  for (const value of ['unknown-option', 5, [question.options[0].id]])
    assert.deepEqual(buildAnswers(document, { [question.id]: value }).errors, {
      [question.id]: '请选择一个有效选项。',
    })
})

test('多选拒绝非数组、数量越界、重复与非法选项', () => {
  const notArray = multipleChoiceQuestion({ min_choices: 1, max_choices: 3 })
  assert.deepEqual(
    buildAnswers(surveyDocument([notArray]), { [notArray.id]: 'not-array' }).errors,
    { [notArray.id]: '请选择一个或多个有效选项。' },
  )
  for (const [question, value] of [
    [
      multipleChoiceQuestion({ min_choices: 2, max_choices: 3 }),
      (question) => [question.options[0].id],
    ],
    [
      multipleChoiceQuestion({ min_choices: 1, max_choices: 1 }),
      (question) => question.options.map((option) => option.id),
    ],
    [
      multipleChoiceQuestion({ min_choices: 1, max_choices: 2 }),
      (question) => [question.options[0].id, question.options[0].id],
    ],
    [
      multipleChoiceQuestion({ min_choices: 1, max_choices: 2 }),
      (question) => [question.options[0].id, 'unknown-option'],
    ],
  ]) {
    const items = value(question)
    assert.deepEqual(buildAnswers(surveyDocument([question]), { [question.id]: items }).errors, {
      [question.id]: `请选择 ${question.min_choices}–${question.max_choices} 项有效选项。`,
    })
  }
})

test('多选合法答案输出独立数组副本', () => {
  const question = multipleChoiceQuestion()
  const value = question.options.map((option) => option.id)
  const result = buildAnswers(surveyDocument([question]), { [question.id]: value })
  assert.deepEqual(result.answers, [{ question_id: question.id, value }])
  assert.notEqual(result.answers[0].value, value)
})

test('单行文本拒绝非字符串、空字符、换行与超长，并按码点计数', () => {
  const question = textQuestion()
  const document = surveyDocument([question])
  const fail = (value, message) =>
    assert.deepEqual(buildAnswers(document, { [question.id]: value }).errors, {
      [question.id]: message,
    })
  fail(7, '文本答案必须是字符串。')
  fail('a\0b', '回答不能包含空字符或无效 Unicode 字符。')
  for (const value of ['a\nb', 'a\rb', 'a\u000bb', 'a\u000cb', 'a\u0085b', 'a\u2028b', 'a\u2029b'])
    fail(value, '单行文本不能包含换行。')
  const short = textQuestion({ id: '00000000-0000-4000-8000-0000000000f2', max_length: 2 })
  assert.deepEqual(buildAnswers(surveyDocument([short]), { [short.id]: 'abc' }).errors, {
    [short.id]: '最多填写 2 个字符。',
  })
  assert.deepEqual(
    buildAnswers(surveyDocument([short]), { [short.id]: '𠀀𠀀' }).errors,
    {},
    '两个补充平面字符按码点计数后未超长',
  )
})

test('文本答案保留首尾空白，必填只拒绝纯空白', () => {
  const question = textQuestion()
  const document = surveyDocument([question])
  assert.deepEqual(buildAnswers(document, { [question.id]: ' 甲 ' }).answers, [
    { question_id: question.id, value: ' 甲 ' },
  ])
  const blank = buildAnswers(document, { [question.id]: ' \t\u3000' })
  assert.deepEqual(blank.answers, [])
  assert.deepEqual(blank.errors, { [question.id]: requiredMessage })
  const optional = textQuestion({ required: false })
  assert.deepEqual(buildAnswers(surveyDocument([optional]), { [optional.id]: '   ' }), {
    answers: [],
    errors: {},
  })
})

test('多行文本允许换行但拒绝空字符与超长', () => {
  const question = textareaQuestion()
  assert.deepEqual(buildAnswers(surveyDocument([question]), { [question.id]: '甲\n乙' }).answers, [
    { question_id: question.id, value: '甲\n乙' },
  ])
  assert.deepEqual(buildAnswers(surveyDocument([question]), { [question.id]: '甲\0乙' }).errors, {
    [question.id]: '回答不能包含空字符或无效 Unicode 字符。',
  })
  const short = textareaQuestion({
    id: '00000000-0000-4000-8000-0000000000f3',
    max_length: 3,
  })
  assert.deepEqual(buildAnswers(surveyDocument([short]), { [short.id]: '甲乙丙丁' }).errors, {
    [short.id]: '最多填写 3 个字符。',
  })
})

test('数字题接受有限数值并执行整数与范围约束', () => {
  const question = numberQuestion()
  const document = surveyDocument([question])
  const answer = (value) => buildAnswers(document, { [question.id]: value })
  for (const [raw, expected] of [
    ['2', 2],
    [3, 3],
    ['1.5', 1.5],
    ['1e3', 1000],
  ])
    assert.deepEqual(
      answer(raw).answers,
      [{ question_id: question.id, value: expected }],
      String(raw),
    )
  const zero = answer('-0')
  assert.deepEqual(zero.errors, {})
  assert.equal(zero.answers.length, 1)
  assert.equal(zero.answers[0].value === 0, true)
  assert.deepEqual(answer('abc').errors, { [question.id]: numbersMessage })
  const integer = numberQuestion({ id: '00000000-0000-4000-8000-0000000000f4', integer: true })
  assert.deepEqual(buildAnswers(surveyDocument([integer]), { [integer.id]: '1.5' }).errors, {
    [integer.id]: '此题只接受整数。',
  })
  const bounded = numberQuestion({
    id: '00000000-0000-4000-8000-0000000000f5',
    min: 1,
    max: 10,
  })
  for (const raw of ['1', '10', '10.0'])
    assert.deepEqual(buildAnswers(surveyDocument([bounded]), { [bounded.id]: raw }).errors, {}, raw)
  for (const raw of ['0', '11', '-1'])
    assert.deepEqual(buildAnswers(surveyDocument([bounded]), { [bounded.id]: raw }).errors, {
      [bounded.id]: '数值不在允许范围内。',
    })
})

test('评分题只接受范围内整数并接受字符串数字', () => {
  const question = ratingQuestion()
  const document = surveyDocument([question])
  for (const [raw, expected] of [
    ['1', 1],
    ['5', 5],
    ['4', 4],
    [5, 5],
  ])
    assert.deepEqual(
      buildAnswers(document, { [question.id]: raw }).answers,
      [{ question_id: question.id, value: expected }],
      String(raw),
    )
  for (const raw of ['3.5', '0', '6', 'abc'])
    assert.deepEqual(
      buildAnswers(document, { [question.id]: raw }).errors,
      {
        [question.id]: '请填写 1–5 之间的整数评分。',
      },
      String(raw),
    )
})
