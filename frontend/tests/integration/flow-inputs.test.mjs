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
})

test('数字过渡前缀暂不判错，失焦结算后才显示校验结果', () => {
  const question = numberQuestion({ required: false })
  const form = new FlowInputs(surveyDocument([question]))
  for (const prefix of ['-', '12.', '1e', '1e-', '.']) {
    form.change(question.id, prefix)
    assert.equal(form.values()[question.id], prefix, `${prefix}：界面保留原文`)
    assert.equal(form.snapshot().errors[question.id], undefined, `${prefix}：击键中途不报错`)
    assert.equal(
      Object.hasOwn(form.snapshot().answers, question.id),
      false,
      `${prefix}：尚未成为正式答案`,
    )
  }
  form.change(question.id, '12.')
  form.commit(question.id)
  assert.equal(form.snapshot().errors[question.id], 'invalid_answer')
  // 完整数字继续正常写回原生数字，负评分可以从单独负号继续输入。
  form.change(question.id, '-')
  form.change(question.id, '-3')
  assert.equal(form.snapshot().answers[question.id], -3)
  assert.equal(form.snapshot().errors[question.id], undefined)
})

test('翻页前自动结算过渡前缀，残缺输入给出题目级错误', () => {
  const question = numberQuestion({ required: false })
  const form = new FlowInputs(surveyDocument([question]))
  form.change(question.id, '12.')
  assert.throws(
    () => form.next(),
    (error) => {
      assert.ok(error instanceof LogicError)
      assert.equal(error.code, 'invalid_answer')
      assert.equal(error.question_id, question.id)
      return true
    },
  )
  assert.equal(form.snapshot().errors[question.id], 'invalid_answer')

  form.change(question.id, '12.5')
  form.next()
  assert.equal(form.snapshot().outcome, 'complete')
  assert.equal(form.submissionAnswers()[0].value, 12.5)
})

test('切换分支后镜像里的已跳过输入一并清理', () => {
  const document = readFlowDocument('diamond')
  const [choice, number, text, merge] = document.questions
  const [first, second] = choice.options
  const form = new FlowInputs(document)
  form.change(choice.id, first.id)
  form.next()
  form.change(number.id, '2')
  form.previous()
  form.change(choice.id, second.id)
  assert.deepEqual(form.snapshot().cleared, [number.id])
  assert.equal(form.values()[number.id], undefined)

  form.next()
  form.change(text.id, '乙路径')
  form.previous()
  form.change(choice.id, first.id)
  assert.deepEqual(form.snapshot().cleared, [text.id])
  form.next()
  assert.equal(form.values()[text.id], undefined)
  form.change(number.id, '1e20')
  form.next()
  form.change(merge.id, '汇合')
  form.next()
  assert.equal(form.snapshot().outcome, 'complete')
  const answers = form.submissionAnswers()
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
  assert.equal(answers[2].value, 5)
  const exported = [...answers[0].value]
  answers[0].value.push('mutated')
  assert.deepEqual(form.submissionAnswers()[0].value, exported, '多选答案数组彼此隔离')
})
