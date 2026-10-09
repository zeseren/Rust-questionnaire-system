//! 创建入口生成的规则必须通过领域编译，并在真实填写会话中保持预期分流。
import assert from 'node:assert/strict'
import test from 'node:test'
import { createSurveyTemplate, surveyTemplates } from '../src/survey-templates.ts'
import { compileDocumentLogic } from '../src/document-logic.ts'
import { compileExpression } from '../src/logic.ts'
import { FlowSession } from '../src/flow.ts'
import { formatLogicText, parseLogicText } from '../src/logic-text.ts'
import {
  conditionExpression,
  groupExpression,
  readConditionGroup,
} from '../src/condition-builder.ts'
import { logicTextSignature, textBindings, textErrorOffset } from '../src/logic-text-tools.ts'

test('模板均可编译并切换文本，每次创建使用独立身份', () => {
  const identities = new Set()
  for (const template of surveyTemplates) {
    for (let generation = 0; generation < 2; generation++) {
      const document = createSurveyTemplate(template.id)
      assert.doesNotThrow(
        () => compileDocumentLogic(document.logic, document.questions),
        template.title,
      )
      const source = formatLogicText(document.logic, document.questions)
      assert.doesNotThrow(() => parseLogicText(source, document.questions), template.title)
      for (const { id } of textBindings(source, document.questions)) {
        assert.ok(!identities.has(id), '新建问卷不能复用其他问卷的配置身份')
        identities.add(id)
      }
    }
  }
})

test('回访模板低分时必填原因，改为高分后清除追问答案', () => {
  const document = createSurveyTemplate('feedback')
  const [score, reason] = document.questions
  const session = new FlowSession(document)
  session.dispatch({ type: 'change', question_id: score.id, value: 2 })
  assert.ok(session.snapshot().visible.includes(reason.id))
  assert.ok(session.snapshot().required.includes(reason.id))
  session.dispatch({ type: 'change', question_id: reason.id, value: '希望减少等待时间' })
  session.dispatch({ type: 'change', question_id: score.id, value: 5 })
  assert.ok(!session.snapshot().visible.includes(reason.id))
  assert.ok(!Object.hasOwn(session.snapshot().answers, reason.id))
  assert.deepEqual(session.snapshot().cleared, [reason.id])
})

test('调研模板按经历分流，改答清理旧路径并汇合到共同页面', () => {
  const document = createSurveyTemplate('routing')
  const [used, experience, expectation, interest] = document.questions
  const session = new FlowSession(document)
  session.dispatch({ type: 'change', question_id: used.id, value: used.options[0].id })
  session.dispatch({ type: 'next' })
  session.dispatch({ type: 'change', question_id: experience.id, value: '容易上手' })
  session.dispatch({ type: 'previous' })
  session.dispatch({ type: 'change', question_id: used.id, value: used.options[1].id })
  assert.deepEqual(session.snapshot().cleared, [experience.id])
  assert.ok(!session.snapshot().visible.includes(experience.id))
  session.dispatch({ type: 'next' })
  session.dispatch({ type: 'change', question_id: expectation.id, value: '减少重复录入' })
  session.dispatch({ type: 'next' })
  session.dispatch({ type: 'change', question_id: interest.id, value: 4 })
  session.dispatch({ type: 'next' })
  assert.equal(session.snapshot().outcome, 'complete')
  assert.deepEqual(
    Object.keys(session.snapshot().answers).sort(),
    [used.id, expectation.id, interest.id].sort(),
  )
})

test('报名模板只在需要接送时要求地点，最终答案不含已隐藏地点', () => {
  const document = createSurveyTemplate('registration')
  const [count, transport, address] = document.questions
  const session = new FlowSession(document)
  session.dispatch({ type: 'change', question_id: count.id, value: 2 })
  session.dispatch({ type: 'change', question_id: transport.id, value: transport.options[0].id })
  assert.ok(session.snapshot().required.includes(address.id))
  assert.throws(() => session.dispatch({ type: 'next' }), {
    code: 'required_answer',
    question_id: address.id,
  })
  session.dispatch({ type: 'change', question_id: address.id, value: '南门' })
  session.dispatch({ type: 'change', question_id: transport.id, value: transport.options[1].id })
  session.dispatch({ type: 'next' })
  assert.equal(session.snapshot().outcome, 'complete')
  assert.ok(!Object.hasOwn(session.snapshot().answers, address.id))
})

test('简洁比较使用当前运算契约，并保留数字原文和任一条件模式', () => {
  const document = createSurveyTemplate('registration')
  const [count] = document.questions
  for (const [operator, expected] of [
    ['eq', true],
    ['ne', false],
    ['gt', false],
    ['ge', true],
    ['lt', false],
    ['le', true],
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
    assert.deepEqual(compiled.evaluate({ [count.id]: 2 }), { type: 'boolean', value: expected })
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
  assert.equal(textErrorOffset(unicode, 999, 999), unicode.length)
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
