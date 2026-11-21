//! 编纂闭环：模板、拓扑插入、题目与规则复制、文本往返后仍可实际填写。
import assert from 'node:assert/strict'
import test from 'node:test'
import { conditionExpression } from '../../src/condition-builder.ts'
import { duplicateQuestion } from '../../src/documents.ts'
import { logicDiagnostic } from '../../src/editor-logic.ts'
import { FlowInputs } from '../../src/flow-inputs.ts'
import { insertFlowNode } from '../../src/flow-tools.ts'
import { createSurveyTemplate } from '../../src/survey-templates.ts'
import { formatLogicText, parseLogicText } from '../../src/wasm/index.ts'

// 从模板到可填写问卷的完整编纂闭环
test('full authoring loop from template to fillable survey', () => {
  const document = createSurveyTemplate('routing')
  const [used, experience, , interest] = document.questions
  const flow = document.logic.flow
  const pageOf = (question) =>
    document.logic.flow.nodes.find(
      (node) => node.type === 'page' && node.question_ids.includes(question.id),
    )

  // 为汇合页题目配置显隐条件；文本往返必须保留这条规则。
  document.logic.rules.push({
    question_id: interest.id,
    visible_if: conditionExpression({ questionId: used.id, operator: 'answered', value: '' }),
  })

  // 复制题目连同自身规则，新题先落在同一页面。
  const copied = duplicateQuestion(interest, document)
  document.questions.push(copied)
  const join = pageOf(interest)
  assert.deepEqual(join.question_ids, [interest.id, copied.id])
  const copiedRule = document.logic.rules.filter((rule) => rule.question_id === copied.id)
  assert.equal(copiedRule.length, 1)
  assert.deepEqual(copiedRule[0].visible_if, { op: 'answered', question_id: used.id })

  // 在汇合页后插入新页面，把复制题移入新页面。
  const pageId = insertFlowNode(flow, flow.nodes.indexOf(join), 'page')
  const inserted = flow.nodes.find((node) => node.id === pageId)
  assert.ok(inserted)
  join.question_ids = [interest.id]
  inserted.question_ids = [copied.id]

  const restored = parseLogicText(
    formatLogicText(document.logic, document.questions),
    document.questions,
  )
  document.logic = restored
  assert.equal(logicDiagnostic(document.logic, document.questions), null)

  // 编纂后的问卷仍可实际填写：使用过的分支汇合到新页面的复制题。
  const form = new FlowInputs(document)
  assert.deepEqual(form.snapshot().visible, [used.id])
  form.change(used.id, used.options[0].id)
  form.next()
  assert.equal(form.snapshot().current_node, pageOf(experience).id)
  form.change(experience.id, '体验')
  form.next()
  assert.equal(form.snapshot().current_node, pageOf(interest).id)
  assert.ok(form.snapshot().visible.includes(interest.id))
  form.change(interest.id, '4')
  form.next()
  assert.equal(form.snapshot().current_node, pageOf(copied).id)
  assert.ok(form.snapshot().visible.includes(copied.id))
  form.change(copied.id, '5')
  form.next()
  assert.equal(form.snapshot().outcome, 'complete')

  const values = Object.fromEntries(
    form.submissionAnswers().map((answer) => [answer.question_id, answer.value]),
  )
  assert.deepEqual(values, {
    [used.id]: used.options[0].id,
    [experience.id]: '体验',
    [interest.id]: 4,
    [copied.id]: 5,
  })
})
