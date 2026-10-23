//! 编纂闭环：模板、拓扑插入、题目与规则复制、文本往返与整卷复制共同协作。
import assert from 'node:assert/strict'
import test from 'node:test'
import { conditionExpression } from '../../src/condition-builder.ts'
import { duplicateQuestion, duplicateSurveyDocument } from '../../src/documents.ts'
import { logicDiagnostic } from '../../src/editor-logic.ts'
import { FlowInputs } from '../../src/flow-inputs.ts'
import { insertFlowNode } from '../../src/flow-tools.ts'
import { createSurveyTemplate } from '../../src/survey-templates.ts'
import { formatLogicText, parseLogicText } from '../../src/wasm/index.ts'

test('从模板到可填写问卷的完整编纂闭环', () => {
  const document = createSurveyTemplate('routing')
  const [used, , , interest] = document.questions
  const flow = document.logic.flow
  const pageOf = (question) =>
    flow.nodes.find((node) => node.type === 'page' && node.question_ids.includes(question.id))

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

  const before = structuredClone(document)
  const duplicated = duplicateSurveyDocument(document)
  assert.deepEqual(document, before, '整卷复制不得修改来源文档')

  // 复制后的整卷仍可实际填写：使用过的分支汇合到新页面的复制题。
  const [dUsed, dExperience, , dInterest, dCopied] = duplicated.questions
  const dFlow = duplicated.logic.flow
  const dPageOf = (question) =>
    dFlow.nodes.find((node) => node.type === 'page' && node.question_ids.includes(question.id))
  const form = new FlowInputs(duplicated)
  assert.deepEqual(form.snapshot().visible, [dUsed.id])
  form.change(dUsed.id, dUsed.options[0].id)
  form.next()
  assert.equal(form.snapshot().current_node, dPageOf(dExperience).id)
  form.change(dExperience.id, '体验')
  form.next()
  assert.equal(form.snapshot().current_node, dPageOf(dInterest).id)
  assert.ok(form.snapshot().visible.includes(dInterest.id))
  form.change(dInterest.id, '4')
  form.next()
  assert.equal(form.snapshot().current_node, dPageOf(dCopied).id)
  assert.ok(form.snapshot().visible.includes(dCopied.id))
  form.change(dCopied.id, '5')
  form.next()
  assert.equal(form.snapshot().outcome, 'complete')

  const answers = form.submissionAnswers()
  const values = Object.fromEntries(answers.map((answer) => [answer.question_id, answer.value]))
  assert.deepEqual(values, {
    [dUsed.id]: dUsed.options[0].id,
    [dExperience.id]: '体验',
    [dInterest.id]: 4,
    [dCopied.id]: 5,
  })
})
