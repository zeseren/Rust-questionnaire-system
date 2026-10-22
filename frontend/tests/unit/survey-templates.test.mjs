//! 内置模板：元数据、空白文档、规则求值与流程行为必须自洽。
import assert from 'node:assert/strict'
import test from 'node:test'
import { createSurveyTemplate, surveyTemplates } from '../../src/survey-templates.ts'
import { FlowSession, compileDocumentLogic } from '../../src/wasm/index.ts'

test('模板元数据是稳定的选择入口', () => {
  assert.deepEqual(
    surveyTemplates.map((template) => template.id),
    ['feedback', 'routing', 'registration'],
  )
  assert.deepEqual(
    surveyTemplates.map((template) => template.title),
    ['满意度回访', '产品体验调研', '活动报名'],
  )
  for (const template of surveyTemplates) {
    assert.ok(template.description.length > 0)
    assert.ok(template.detail.length > 0)
    assert.ok(template.icon.length > 0)
    assert.ok(template.color.length > 0)
  }
})

test('不选模板时生成不含规则的空文档', () => {
  assert.deepEqual(createSurveyTemplate(null), {
    title: '',
    description: '',
    purpose: '',
    thank_you: '感谢你的认真回答。',
    questions: [],
  })
})

test('满意度回访按低分追问并传播缺失值', () => {
  const document = createSurveyTemplate('feedback')
  assert.deepEqual(
    document.questions.map((question) => [question.type, question.title]),
    [
      ['rating', '你对本次服务的整体体验满意吗？'],
      ['textarea', '哪些地方还可以做得更好？'],
      ['single_choice', '你愿意将我们的服务推荐给朋友吗？'],
    ],
  )
  const [score, reason] = document.questions
  assert.equal(reason.required, false)
  const variable = document.logic.variables[0]
  const compiled = compileDocumentLogic(document.logic, document.questions)
  assert.deepEqual(compiled.variableOrder, [variable.id])
  for (const [answer, expected] of [
    [1, true],
    [3, true],
    [4, false],
    [5, false],
  ]) {
    const values = compiled.evaluate({ [score.id]: answer })
    assert.deepEqual(values.variables[variable.id], { type: 'boolean', value: expected })
    assert.deepEqual(values.rules[reason.id], {
      visible_if: { type: 'boolean', value: expected },
      required_if: { type: 'boolean', value: expected },
      validate: null,
    })
  }
  const missing = compiled.evaluate({})
  assert.deepEqual(missing.rules[reason.id], {
    visible_if: { type: 'missing' },
    required_if: { type: 'missing' },
    validate: null,
  })
})

test('产品体验调研按使用经历分流并在结束页汇合', () => {
  const document = createSurveyTemplate('routing')
  const [used, experience, expectation, interest] = document.questions
  const flow = document.logic.flow
  const pageOf = (question) =>
    flow.nodes.find((node) => node.type === 'page' && node.question_ids.includes(question.id))
  const returning = pageOf(experience)
  const newcomer = pageOf(expectation)
  const join = pageOf(interest)
  assert.ok(returning && newcomer && join)
  assert.equal(flow.nodes[0].id, flow.entry)
  const compiled = compileDocumentLogic(document.logic, document.questions)
  assert.ok(Array.isArray(compiled.variableOrder))
  assert.equal(compiled.variableOrder.length, 0, '分流模板不声明计算变量')

  const branch = (option) => {
    const session = new FlowSession(document)
    session.dispatch({ type: 'change', question_id: used.id, value: option.id })
    session.dispatch({ type: 'next' })
    return session.snapshot()
  }
  const usedPath = branch(used.options[0])
  assert.equal(usedPath.current_node, returning.id)
  assert.ok(usedPath.visible.includes(experience.id), '使用过的分支显示体验题')
  assert.ok(!usedPath.visible.includes(expectation.id), '未使用的分支不显示期待题')
  const newPath = branch(used.options[1])
  assert.equal(newPath.current_node, newcomer.id)
  assert.ok(newPath.visible.includes(expectation.id), '未使用过的分支显示期待题')
  assert.ok(!newPath.visible.includes(experience.id), '使用过的分支不显示体验题')

  const session = new FlowSession(document)
  session.dispatch({ type: 'change', question_id: used.id, value: used.options[0].id })
  session.dispatch({ type: 'next' })
  session.dispatch({ type: 'change', question_id: experience.id, value: '记得很清楚' })
  session.dispatch({ type: 'next' })
  assert.equal(session.snapshot().current_node, join.id)
  assert.ok(session.snapshot().visible.includes(interest.id), '汇合页显示共同题目')
  session.dispatch({ type: 'change', question_id: interest.id, value: 4 })
  session.dispatch({ type: 'next' })
  assert.equal(session.snapshot().outcome, 'complete')
})

test('活动报名约束人数并仅按接送需求显示地址', () => {
  const document = createSurveyTemplate('registration')
  const [count, transport, address] = document.questions
  assert.equal(count.type, 'number')
  assert.deepEqual([count.integer, count.min, count.max], [true, 1, 10])

  const compiled = compileDocumentLogic(document.logic, document.questions)
  const selected = compiled.evaluate({ [transport.id]: transport.options[0].id })
  assert.deepEqual(selected.rules[address.id], {
    visible_if: { type: 'boolean', value: true },
    required_if: { type: 'boolean', value: true },
    validate: null,
  })
  const self = compiled.evaluate({ [transport.id]: transport.options[1].id })
  assert.deepEqual(self.rules[address.id], {
    visible_if: { type: 'boolean', value: false },
    required_if: { type: 'boolean', value: false },
    validate: null,
  })

  const session = new FlowSession(document)
  for (const [value, code] of [
    [0, 'invalid_answer'],
    [1, undefined],
    [11, 'invalid_answer'],
  ]) {
    session.dispatch({ type: 'change', question_id: count.id, value })
    assert.equal(session.snapshot().errors[count.id], code, String(value))
  }
  const visibility = new FlowSession(document)
  visibility.dispatch({ type: 'change', question_id: transport.id, value: transport.options[0].id })
  assert.ok(visibility.snapshot().visible.includes(address.id))
  assert.equal(visibility.snapshot().errors[address.id], 'required_answer')
  visibility.dispatch({ type: 'change', question_id: transport.id, value: transport.options[1].id })
  assert.ok(!visibility.snapshot().visible.includes(address.id))
  assert.equal(visibility.snapshot().errors[address.id], undefined)
})

test('每次创建模板都使用全新配置身份', () => {
  const identities = new Set()
  for (const template of surveyTemplates) {
    for (let generation = 0; generation < 2; generation++) {
      const document = createSurveyTemplate(template.id)
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

test('未知模板立即拒绝', () => {
  assert.throws(() => createSurveyTemplate('unknown'), { message: '未知问卷模板' })
})
