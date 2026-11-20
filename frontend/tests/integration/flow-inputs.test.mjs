//! 流程输入适配器：界面原文与引擎原生值之间的边界协作。
import assert from 'node:assert/strict'
import test from 'node:test'
import { FlowInputs, replayEvents } from '../../src/flow-inputs.ts'
import { LogicError } from '../../src/logic.ts'
import { readFlowDocument } from '../helpers/fixtures.mjs'
import {
  numberQuestion,
  ratingQuestion,
  surveyDocument,
  textQuestion,
} from '../helpers/documents.mjs'

const logicFailure = (code, questionId) => (error) => {
  assert.ok(error instanceof LogicError)
  assert.equal(error.code, code)
  assert.equal(error.question_id, questionId)
  return true
}

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
  for (const prefix of ['-', '12.', '1e', '1e-', '1.5e+']) {
    form.change(question.id, prefix)
    assert.equal(form.values()[question.id], prefix, `${prefix}：界面保留原文`)
    assert.equal(form.snapshot().errors[question.id], undefined, `${prefix}：击键中途不报错`)
    assert.equal(
      Object.hasOwn(form.snapshot().answers, question.id),
      false,
      `${prefix}：尚未成为正式答案`,
    )
    assert.ok(form.unsettled().has(question.id), `${prefix}：标记为未结算`)
  }
  form.change(question.id, '12.')
  form.commit(question.id)
  assert.equal(form.snapshot().errors[question.id], 'invalid_answer')
  assert.equal(form.unsettled().has(question.id), false, '结算后错误属于当前输入，不再隐藏')
  // 完整数字继续正常写回原生数字，负评分可以从单独负号继续输入。
  form.change(question.id, '-')
  form.change(question.id, '-3')
  assert.equal(form.snapshot().answers[question.id], -3)
  assert.equal(form.snapshot().errors[question.id], undefined)
})

test('永远补不成合法数字的写法不算输入中，立即交给引擎判错', () => {
  const question = numberQuestion({ required: false })
  for (const raw of ['.', '.5', '-.5', '00', '01', '1.e']) {
    const form = new FlowInputs(surveyDocument([question]))
    form.change(question.id, raw)
    assert.equal(form.values()[question.id], raw, `${raw}：界面保留原文`)
    assert.equal(form.snapshot().errors[question.id], 'invalid_answer', raw)
    assert.equal(form.unsettled().size, 0, `${raw}：错误不被当作输入中隐藏`)
  }
})

test('翻页前自动结算过渡前缀，残缺输入给出题目级错误', () => {
  const question = numberQuestion({ required: false })
  const form = new FlowInputs(surveyDocument([question]))
  form.change(question.id, '12.')
  assert.throws(() => form.next(), logicFailure('invalid_answer', question.id))
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

test('未配置流程的问卷缺必填时拒绝前进，补全后可提交并退回编辑页', () => {
  const required = textQuestion()
  const optional = numberQuestion({ required: false })
  const form = new FlowInputs(surveyDocument([required, optional]))
  assert.throws(() => form.next(), logicFailure('required_answer', required.id))
  assert.equal(form.snapshot().outcome, null, '校验失败留在原页')

  form.change(required.id, '补全后的回答')
  form.next()
  assert.equal(form.snapshot().outcome, 'complete')
  assert.deepEqual(form.submissionAnswers(), [{ question_id: required.id, value: '补全后的回答' }])

  form.previous()
  assert.equal(form.snapshot().outcome, null, '提交后回到可编辑页')
  assert.equal(form.values()[required.id], '补全后的回答')
})

test('合法暂存进度按真实会话完整回放', () => {
  const document = readFlowDocument('diamond')
  const [choice, number, , merge] = document.questions
  const form = new FlowInputs(document)
  const events = [
    { type: 'change', question_id: choice.id, value: choice.options[0].id },
    { type: 'next' },
    { type: 'change', question_id: number.id, value: '2' },
    { type: 'next' },
    { type: 'change', question_id: merge.id, value: '汇合' },
  ]
  const result = replayEvents(form, events, false)
  assert.equal(result.applied, events.length)
  assert.equal(result.total, events.length)
  assert.deepEqual(result.events, events)
  assert.equal(form.snapshot().outcome, null, '回放不替用户确认终点')
  assert.equal(form.values()[merge.id], '汇合')
})

test('回放在首个不可应用事件处停止并保留成功前缀', () => {
  const document = readFlowDocument('diamond')
  const [choice, number, skipped] = document.questions
  const form = new FlowInputs(document)
  const result = replayEvents(
    form,
    [
      { type: 'change', question_id: choice.id, value: choice.options[0].id },
      { type: 'next' },
      { type: 'change', question_id: number.id, value: '2' },
      { type: 'change', question_id: skipped.id, value: '被分支跳过' },
      { type: 'next' },
    ],
    false,
  )
  assert.equal(result.applied, 3, '跳过页的改答不能算成功')
  assert.equal(result.total, 5)
  assert.deepEqual(
    result.events.map((event) => event.type),
    ['change', 'next', 'change'],
  )
  assert.equal(form.values()[skipped.id], undefined)
})

test('过渡数字前缀只接受当前页的题目', () => {
  const document = readFlowDocument('diamond')
  const [choice, number] = document.questions
  const future = new FlowInputs(document)
  const rejected = replayEvents(
    future,
    [
      { type: 'change', question_id: choice.id, value: choice.options[0].id },
      { type: 'change', question_id: number.id, value: '-' },
    ],
    false,
  )
  assert.ok(
    future.snapshot().visible.includes(number.id),
    '分支已解析时后续页题目仍可能出现在投影里',
  )
  assert.equal(rejected.applied, 1, '尚未到达的数字题不能借过渡前缀早退绕过引擎')
  assert.equal(future.values()[number.id], undefined)
  assert.equal(future.unsettled().size, 0)

  const current = new FlowInputs(document)
  const accepted = replayEvents(
    current,
    [
      { type: 'change', question_id: choice.id, value: choice.options[0].id },
      { type: 'next' },
      { type: 'change', question_id: number.id, value: '-' },
    ],
    false,
  )
  assert.equal(accepted.applied, 3, '合法当前页的负号仍作为过渡输入恢复')
  assert.equal(current.values()[number.id], '-')
  assert.equal(current.unsettled().has(number.id), true)
})

test('首个事件不可应用时 applied 为 0 且不改变会话', () => {
  const document = readFlowDocument('diamond')
  const form = new FlowInputs(document)
  const before = form.snapshot()
  const result = replayEvents(
    form,
    [
      { type: 'next' },
      {
        type: 'change',
        question_id: document.questions[0].id,
        value: document.questions[0].options[0].id,
      },
    ],
    false,
  )
  assert.equal(result.applied, 0, '必填未答的导航失败不算回放')
  assert.equal(result.total, 2)
  assert.deepEqual(result.events, [])
  assert.deepEqual(form.snapshot(), before, '失败事件不修改会话')

  const malformed = replayEvents(
    new FlowInputs(document),
    [{ type: 'change', question_id: document.questions[0].id, value: ['not', 'a', 1] }],
    false,
  )
  assert.equal(malformed.applied, 0, '数组里混入非字符串时拒绝整个值')
})

test('普通问卷只接受改答，非法导航与损坏事件不计数', () => {
  const document = readFlowDocument('plain')
  const text = document.questions[2].id
  const form = new FlowInputs(document)
  const result = replayEvents(
    form,
    [
      { type: 'change', question_id: text, value: '保留' },
      { type: 'next' },
      { type: 'change', question_id: text, value: '丢弃' },
    ],
    true,
  )
  assert.equal(result.applied, 1)
  assert.equal(result.total, 3)
  assert.equal(form.values()[text], '保留', '失败后的事件不再应用')

  const broken = new FlowInputs(readFlowDocument('plain'))
  const damaged = replayEvents(
    broken,
    [
      { type: 'change', question_id: text, value: '有效' },
      { type: 'change', question_id: text },
      { type: 'change', question_id: text, value: { nested: true } },
    ],
    true,
  )
  assert.equal(damaged.applied, 1, '形状无效的事件立即停止')
  assert.deepEqual(damaged.events, [{ type: 'change', question_id: text, value: '有效' }])
})

test('无答案但成功回放与领域标错的有效输入都算已回放', () => {
  const number = numberQuestion({ required: false })
  const form = new FlowInputs(surveyDocument([number]))
  const transient = replayEvents(
    form,
    [{ type: 'change', question_id: number.id, value: '12.' }],
    true,
  )
  assert.equal(transient.applied, 1)
  assert.equal(form.unsettled().has(number.id), true)
  assert.deepEqual(form.snapshot().answers, {}, '过渡前缀不产生正式答案')

  const short = textQuestion({ max_length: 3 })
  const wrong = new FlowInputs(surveyDocument([short]))
  const marked = replayEvents(
    wrong,
    [{ type: 'change', question_id: short.id, value: '四个字符' }],
    true,
  )
  assert.equal(marked.applied, 1, '领域标错不是事件形状损坏')
  assert.equal(wrong.snapshot().errors[short.id], 'invalid_answer')
})

test('快照为题级限制给出可直接展示的修正文案', () => {
  const text = textQuestion({ max_length: 3 })
  const number = numberQuestion({ min: 1, max: 5 })
  const form = new FlowInputs(surveyDocument([text, number]))
  form.change(text.id, '四个字符')
  assert.equal(form.snapshot().errors[text.id], 'invalid_answer')
  const textMessage = form.snapshot().error_messages[text.id]
  assert.match(textMessage, /3/, '超长文案带上真实上限')
  form.change(number.id, '6')
  assert.equal(form.snapshot().errors[number.id], 'invalid_answer')
  assert.match(form.snapshot().error_messages[number.id], /范围|介于|之间|1|5/)
})
