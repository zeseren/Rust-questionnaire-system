//! 流程输入适配器：界面原文与引擎原生值之间的边界协作。
import assert from 'node:assert/strict'
import test from 'node:test'
import { FlowInputs } from '../../src/flow-inputs.ts'
import { LogicError } from '../../src/logic.ts'
import { readFlowDocument } from '../helpers/fixtures.mjs'
import { numberQuestion, ratingQuestion, surveyDocument } from '../helpers/documents.mjs'

test('评分原文转成原生数字，非法整数交给引擎报错', () => {
  const question = ratingQuestion({ required: false })
  const form = new FlowInputs(surveyDocument([question]))
  form.change(question.id, '4')
  assert.equal(form.values()[question.id], '4', '界面保留原始字符串')
  assert.equal(form.snapshot().answers[question.id], 4)
  form.change(question.id, '4.5')
  assert.equal(form.snapshot().errors[question.id], 'invalid_answer')
  assert.equal(form.values()[question.id], '4.5')
})

test('空串转成缺失值，非法数字原文透传给引擎', () => {
  const question = numberQuestion({ required: false })
  const form = new FlowInputs(surveyDocument([question]))
  form.change(question.id, '   ')
  assert.equal(form.values()[question.id], '   ')
  assert.equal(Object.hasOwn(form.snapshot().answers, question.id), false)
  assert.equal(form.snapshot().errors[question.id], undefined)
  form.change(question.id, 'abc')
  assert.equal(form.values()[question.id], 'abc')
  assert.equal(form.snapshot().errors[question.id], 'invalid_answer')
  form.change(question.id, '1e400')
  assert.equal(form.values()[question.id], '1e400')
  assert.equal(form.snapshot().errors[question.id], 'invalid_answer')
})

test('切换分支后镜像里的已跳过输入与错误一并清理', () => {
  const document = readFlowDocument('diamond')
  const [choice, number, text, merge] = document.questions
  const [first, second] = choice.options
  const form = new FlowInputs(document)
  form.change(choice.id, first.id)
  form.next()
  form.change(number.id, 'invalid')
  assert.equal(form.snapshot().errors[number.id], 'invalid_answer')
  assert.throws(
    () => form.next(),
    (error) => error instanceof LogicError && error.code === 'invalid_answer',
  )
  form.previous()
  form.change(choice.id, second.id)
  assert.deepEqual(form.snapshot().cleared, [number.id])
  assert.equal(form.values()[number.id], undefined)
  assert.equal(form.snapshot().errors[number.id], undefined)

  form.next()
  form.change(text.id, '乙路径')
  form.previous()
  form.change(choice.id, first.id)
  assert.deepEqual(form.snapshot().cleared, [text.id])
  form.next()
  assert.equal(form.values()[text.id], undefined)
  form.change(number.id, '1e20')
  form.next()
  assert.ok(form.snapshot().required.includes(merge.id))
  assert.throws(
    () => form.submissionAnswers(),
    (error) => error instanceof LogicError && error.code === 'invalid_event',
    '到达终点前不可提交',
  )
  form.change(merge.id, '汇合')
  form.next()
  const answers = form.submissionAnswers()
  assert.deepEqual(
    answers.map((answer) => answer.question_id),
    [choice.id, number.id, merge.id],
  )
  assert.equal(answers.find((answer) => answer.question_id === number.id).value, 1e20)
})

test('最终答案按题目字典序导出且数组彼此隔离', () => {
  const document = readFlowDocument('plain')
  const [, multiple, text, , , rating] = document.questions
  const reversed = [multiple.options[1].id, multiple.options[0].id]
  const form = new FlowInputs(document)
  form.change(multiple.id, reversed)
  form.change(text.id, '正文')
  form.change(rating.id, '5')
  assert.deepEqual(form.values()[multiple.id], reversed, '界面镜像保留输入顺序')
  form.next()
  assert.equal(form.snapshot().outcome, 'complete')
  const answers = form.submissionAnswers()
  assert.deepEqual(
    answers.map((answer) => answer.question_id),
    [multiple.id, text.id, rating.id],
  )
  assert.deepEqual(
    answers[0].value,
    [multiple.options[0].id, multiple.options[1].id],
    '引擎按选项身份规范化顺序',
  )
  assert.equal(answers[2].value, 5)
  answers[0].value.push('mutated')
  assert.deepEqual(form.submissionAnswers()[0].value, [
    multiple.options[0].id,
    multiple.options[1].id,
  ])
})
