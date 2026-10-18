//! 编辑器回归守住配置身份、条件转换和已应用 DSL 的同步状态。
import assert from 'node:assert/strict'
import test from 'node:test'
import { createSurveyTemplate, surveyTemplates } from '../src/survey-templates.ts'
import { compileDocumentLogic, compileExpression, formatLogicText } from '../src/wasm/index.ts'
import {
  changeConditionOperator,
  conditionExpression,
  groupExpression,
  readConditionGroup,
} from '../src/condition-builder.ts'
import { logicTextSignature, textBindings, textErrorOffset } from '../src/logic-text-tools.ts'

test('模板配置有效且每次创建使用独立身份', () => {
  const identities = new Set()
  for (const template of surveyTemplates) {
    for (let generation = 0; generation < 2; generation++) {
      const document = createSurveyTemplate(template.id)
      assert.doesNotThrow(
        () => compileDocumentLogic(document.logic, document.questions),
        template.title,
      )
      const ids = [
        ...document.questions.flatMap((question) => [
          question.id,
          ...(question.options ?? []).map((option) => option.id),
        ]),
        ...document.logic.variables.map((variable) => variable.id),
        ...(document.logic.flow?.nodes ?? []).map((node) => node.id),
      ]
      for (const id of ids) {
        assert.ok(!identities.has(id), '新建问卷不能复用其他问卷的配置身份')
        identities.add(id)
      }
    }
  }
})

test('简洁比较使用当前运算契约，并保留数字原文和任一条件模式', () => {
  const document = createSurveyTemplate('registration')
  const [count] = document.questions
  for (const [operator, answer, expected] of [
    ['ge', 1, false],
    ['ge', 2, true],
    ['le', 2, true],
    ['le', 3, false],
  ]) {
    const expression = conditionExpression({
      questionId: count.id,
      operator,
      value: '2.00',
      valueType: 'number',
    })
    const compiled = compileExpression(
      { dsl_version: 1, semantics_version: 1, expression },
      document.questions,
    )
    assert.deepEqual(compiled.evaluate({ [count.id]: answer }), {
      type: 'boolean',
      value: expected,
    })
    assert.equal(readConditionGroup(expression).conditions[0].value, '2.00')
  }
  const group = {
    mode: 'any',
    conditions: [{ questionId: count.id, operator: 'answered', value: '' }],
  }
  assert.equal(
    readConditionGroup(groupExpression(group)).mode,
    'any',
    '只有一个条件时也保留作者选择的组合方式',
  )
  assert.equal(
    readConditionGroup({ op: 'all', expressions: [{ op: 'all', expressions: [] }] }),
    null,
    '嵌套表达式不能被静默改写',
  )
  assert.equal(
    readConditionGroup({ op: 'answered', question_id: count.id, extra: true }),
    null,
    '不能丢弃未知字段来掩盖无效配置',
  )
})

test('选项条件保持选项身份，数字和文本条件不互相转换', () => {
  const document = createSurveyTemplate('registration')
  const [, transport, address] = document.questions
  const condition = {
    questionId: transport.id,
    operator: 'not_selected',
    value: transport.options[0].id,
  }
  const expression = conditionExpression(condition)
  assert.deepEqual(readConditionGroup(expression).conditions, [condition])
  const text = conditionExpression({
    questionId: address.id,
    operator: 'eq',
    value: '001',
    valueType: 'text',
  })
  const compiled = compileExpression(
    { dsl_version: 1, semantics_version: 1, expression: text },
    document.questions,
  )
  assert.deepEqual(compiled.evaluate({ [address.id]: '001' }), { type: 'boolean', value: true })
  assert.deepEqual(compiled.evaluate({ [address.id]: '1' }), { type: 'boolean', value: false })
})

test('切换同类运算符保留数字阈值、文本和选项身份', () => {
  const document = createSurveyTemplate('registration')
  const [number, choice, text] = document.questions
  const cases = [
    {
      question: number,
      condition: { questionId: number.id, operator: 'ge', value: '18.00', valueType: 'number' },
      operator: 'gt',
      answers: [
        [18, false],
        [19, true],
      ],
    },
    {
      question: text,
      condition: { questionId: text.id, operator: 'eq', value: '001', valueType: 'text' },
      operator: 'ne',
      answers: [
        ['001', false],
        ['1', true],
      ],
    },
    {
      question: choice,
      condition: { questionId: choice.id, operator: 'selected', value: choice.options[1].id },
      operator: 'not_selected',
      answers: [
        [choice.options[0].id, true],
        [choice.options[1].id, false],
      ],
    },
  ]
  for (const item of cases) {
    const changed = changeConditionOperator(item.condition, item.operator, item.question)
    assert.equal(changed.value, item.condition.value, '只修改运算符不能改写已有操作数')
    const compiled = compileExpression(
      { dsl_version: 1, semantics_version: 1, expression: conditionExpression(changed) },
      document.questions,
    )
    for (const [answer, expected] of item.answers)
      assert.deepEqual(compiled.evaluate({ [item.question.id]: answer }), {
        type: 'boolean',
        value: expected,
      })
  }
})

test('文本辅助忽略注释和字符串中的伪绑定，按 Unicode 列定位', () => {
  const document = createSurveyTemplate('feedback')
  const source = formatLogicText(document.logic, document.questions)
  const prefixed = `// bind question fake = "伪身份";\n${source}\n// "注释中的字符串"\n`
  const bindings = textBindings(prefixed, document.questions)
  assert.ok(!bindings.some((binding) => binding.alias === 'fake'))
  assert.equal(
    bindings.find((binding) => binding.alias === 'q1').label,
    document.questions[0].title,
  )
  assert.deepEqual(textBindings('"bind question fake = \\"伪身份\\";"', []), [])
  const unicode = '// 😀\nvalidate q1 = "𠀀";'
  const offset = textErrorOffset(unicode, 2, 18)
  assert.equal(unicode.slice(offset), ';')
})

test('保存响应的字段顺序与可选空值不使已应用文本变脏', () => {
  const document = createSurveyTemplate('feedback')
  const original = logicTextSignature(document.logic, document.questions)
  const response = {
    flow: null,
    rules: document.logic.rules.map((rule) => ({ validate: null, ...rule })),
    variables: document.logic.variables.map((variable) => ({
      expression: variable.expression,
      value_type: variable.value_type,
      id: variable.id,
    })),
    semantics_version: 1,
    dsl_version: 1,
  }
  assert.equal(logicTextSignature(response, document.questions), original)
  response.variables[0].expression.right.value = '2'
  assert.notEqual(
    logicTextSignature(response, document.questions),
    original,
    '实际规则修改仍需提示同步',
  )
})
