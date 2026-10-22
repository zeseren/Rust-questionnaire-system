//! `src/wasm/index.ts` 包装层契约：信封成功值解码、错误信封还原为领域错误类，
//! 以及有状态 `FlowSession` 的失败注入。语义本身由 `logic-core` 测试与契约夹具覆盖。
import assert from 'node:assert/strict'
import test from 'node:test'
import { LogicError, LogicTextError } from '../../src/logic.ts'
import {
  FlowSession,
  compileDocumentLogic,
  compileExpression,
  formatLogicText,
  parseLogicText,
  renewDocumentIds,
} from '../../src/wasm/index.ts'

const Q_NAME = '00000000-0000-4000-8000-000000000001'
const Q_SCORE = '00000000-0000-4000-8000-000000000002'
const VARIABLE = '00000000-0000-4000-8000-000000002001'
const UNKNOWN_QUESTION = '00000000-0000-4000-8000-000000000999'
const OPTION_A = '00000000-0000-4000-8000-000000000101'
const OPTION_B = '00000000-0000-4000-8000-000000000102'

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

test('compileExpression 返回静态类型、依赖集合与可求值函数', () => {
  const compiled = compileExpression(
    {
      dsl_version: 1,
      semantics_version: 1,
      expression: {
        op: 'arithmetic',
        operator: 'add',
        left: { op: 'answer', question_id: Q_SCORE },
        right: { op: 'number', value: '2' },
      },
    },
    questions,
  )

  assert.equal(compiled.valueType, 'number')
  assert.deepEqual(compiled.dependencies, [Q_SCORE])
  assert.deepEqual(compiled.evaluate({ [Q_SCORE]: 40 }), { type: 'number', value: 42 })
})

test('compileExpression 拒绝计算变量引用并定位到表达式根', () => {
  assert.throws(
    () =>
      compileExpression(
        {
          dsl_version: 1,
          semantics_version: 1,
          expression: { op: 'variable', variable_id: VARIABLE },
        },
        questions,
      ),
    (error) => {
      assert.ok(error instanceof LogicError)
      assert.equal(error.code, 'invalid_reference')
      assert.equal(error.path, '$.expression')
      assert.equal(error.question_id, VARIABLE)
      return true
    },
  )
})

test('compileDocumentLogic 返回变量拓扑顺序并可按键求值', () => {
  const logic = {
    dsl_version: 1,
    semantics_version: 1,
    variables: [
      {
        id: VARIABLE,
        value_type: 'number',
        expression: { op: 'answer', question_id: Q_SCORE },
      },
    ],
    rules: [
      {
        question_id: Q_SCORE,
        validate: {
          op: 'compare',
          comparison: 'ge',
          left: { op: 'answer', question_id: Q_SCORE },
          right: { op: 'number', value: '0' },
        },
      },
    ],
  }

  const compiled = compileDocumentLogic(logic, questions)
  assert.deepEqual(compiled.variableOrder, [VARIABLE])
  assert.deepEqual(compiled.evaluate({ [Q_SCORE]: 5 }), {
    variables: { [VARIABLE]: { type: 'number', value: 5 } },
    rules: {
      [Q_SCORE]: {
        visible_if: null,
        required_if: null,
        validate: { type: 'boolean', value: true },
      },
    },
  })
})

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
      assert.equal(error.message, '规则结构或字段无效（第 1 行，第 1 列）')
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

test('FlowSession 快照、改答、非法回退与最终答案重建遵循契约', () => {
  const session = new FlowSession(sessionDocument)

  const initial = session.snapshot()
  assert.deepEqual(initial.visible, [Q_NAME])
  assert.deepEqual(initial.answers, {})
  assert.deepEqual(initial.errors, { [Q_NAME]: 'required_answer' })
  assert.equal(initial.outcome, null)

  session.dispatch({ type: 'change', question_id: Q_NAME, value: '张三' })
  const changed = session.snapshot()
  assert.equal(changed.answers[Q_NAME], '张三')
  assert.deepEqual(changed.errors, {})

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

  const evaluated = session.evaluateAnswers([{ question_id: Q_NAME, value: '张三' }])
  assert.equal(evaluated.answers[Q_NAME], '张三')
  assert.equal(evaluated.question_states[Q_NAME], 'answered')
  assert.ok(evaluated.path.length > 0)
})

test('renewDocumentIds 返回重写全部身份后的新文档，且不修改入参', () => {
  const original = {
    title: '复制来源',
    description: '',
    purpose: '验证身份重写',
    thank_you: '',
    questions: [
      {
        id: Q_NAME,
        title: '选择',
        required: false,
        type: 'single_choice',
        options: [
          { id: OPTION_A, label: '甲' },
          { id: OPTION_B, label: '乙' },
        ],
      },
    ],
  }

  const renewed = renewDocumentIds(original)
  const question = renewed.questions[0]
  const generated = [question.id, question.options[0].id, question.options[1].id]

  assert.equal(renewed.title, '复制来源')
  assert.equal(question.title, '选择')
  assert.equal(question.options[0].label, '甲')
  for (const value of generated)
    assert.match(value, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/)
  assert.equal(new Set(generated).size, generated.length)
  assert.notEqual(question.id, Q_NAME)
  assert.notEqual(question.options[0].id, OPTION_A)
  assert.notEqual(question.options[1].id, OPTION_B)

  // 入参只是被编码后传入 wasm，返回的是独立对象。
  assert.equal(original.questions[0].id, Q_NAME)
  assert.equal(original.questions[0].options[0].id, OPTION_A)
})
