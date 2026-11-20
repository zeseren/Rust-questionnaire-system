//! 内置模板：空白文档、规则求值与流程行为必须自洽。
import assert from 'node:assert/strict'
import test from 'node:test'
import { createSurveyTemplate, surveyTemplates } from '../../src/survey-templates.ts'
import { FlowSession, compileDocumentLogic } from '../../src/wasm/index.ts'

test('不选模板时生成不含规则的空文档', () => {
  const document = createSurveyTemplate(null)
  assert.deepEqual(document.questions, [])
  assert.equal(document.logic, undefined)
})

test('满意度回访按低分追问并传播缺失值', () => {
  const document = createSurveyTemplate('feedback')
  assert.deepEqual(
    document.questions.map((question) => question.type),
    ['rating', 'textarea', 'single_choice'],
  )
  const [score, reason] = document.questions
  assert.equal(reason.required, false)
  const variable = document.logic.variables[0]
  const compiled = compileDocumentLogic(document.logic, document.questions)
  assert.deepEqual(compiled.variableOrder, [variable.id])

  const feedback = (answer) => {
    const session = new FlowSession(document)
    if (answer !== undefined)
      session.dispatch({ type: 'change', question_id: score.id, value: answer })
    return session.snapshot()
  }

  // 低分追问：追问可见且动态必填，未填写时保留必填错误。
  const low = feedback(3)
  assert.ok(low.visible.includes(reason.id))
  assert.ok(low.required.includes(reason.id))
  assert.equal(low.errors[reason.id], 'required_answer')

  // 高分不追问：隐藏题目既不进必填集合，也不留下错误。
  const high = feedback(4)
  assert.ok(!high.visible.includes(reason.id))
  assert.ok(!high.required.includes(reason.id))
  assert.equal(high.errors[reason.id], undefined)

  // 未作答的评分让条件取缺失值：追问同样不可见且不产生错误。
  const missing = feedback(undefined)
  assert.ok(!missing.visible.includes(reason.id))
  assert.ok(!missing.required.includes(reason.id))
  assert.equal(missing.errors[reason.id], undefined)
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
  assert.notEqual(returning.id, newcomer.id)
  assert.equal(flow.nodes[0].id, flow.entry)
  const compiled = compileDocumentLogic(document.logic, document.questions)
  assert.equal(compiled.variableOrder.length, 0, '分流模板不声明计算变量')

  const branch = (option) => {
    const session = new FlowSession(document)
    session.dispatch({ type: 'change', question_id: used.id, value: option.id })
    session.dispatch({ type: 'next' })
    return session.snapshot()
  }
  assert.equal(branch(used.options[0]).current_node, returning.id)
  assert.equal(branch(used.options[1]).current_node, newcomer.id)

  // 两个分支页都必须在同一结束页汇合。
  assert.equal(returning.next, join.id)
  assert.equal(newcomer.next, join.id)
})

test('活动报名约束人数并仅按接送需求显示地址', () => {
  const document = createSurveyTemplate('registration')
  const [count, transport, address] = document.questions
  assert.equal(count.type, 'number')
  assert.deepEqual([count.integer, count.min, count.max], [true, 1, 10])

  const session = new FlowSession(document)
  session.dispatch({ type: 'change', question_id: count.id, value: 0 })
  assert.equal(session.snapshot().errors[count.id], 'invalid_answer')
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
  assert.throws(() => createSurveyTemplate('unknown'))
})
