//! 流程语义共用人工预期，确保分支改答清理与旧问卷行为不漂移。
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import { copy, decode } from '../src/json.ts'
import { compileDocumentLogic, duplicateSurveyDocument } from '../src/document-logic.ts'
import { duplicateQuestion } from '../src/documents.ts'
import { FlowSession } from '../src/flow.ts'
import { LogicError } from '../src/logic.ts'

const fixtures = JSON.parse(
  readFileSync(new URL('../../fixtures/survey-logic/flows.json', import.meta.url), 'utf8'),
)

test('配置与作答题目身份拒绝非规范 UUID 且选项值仍规范化', () => {
  const contracts = JSON.parse(
    readFileSync(
      new URL('../../fixtures/survey-logic/input-contracts.json', import.meta.url),
      'utf8',
    ),
  )
  const source = fixtures.documents[contracts.document]
  for (const path of contracts.configuration_paths) {
    const segments = path.slice(1).split('/')
    const original = segments.reduce((value, key) => value[key], source)
    // 同步重命名声明和引用后，只扰动一个字段，避免其他字段掩盖校验缺口。
    const renamed = JSON.stringify(source).replaceAll(original, contracts.canonical_id)
    for (const item of contracts.uuid_cases) {
      const doc = JSON.parse(renamed)
      segments.slice(0, -1).reduce((value, key) => value[key], doc)[segments.at(-1)] = item.raw
      let valid = true
      try {
        compileDocumentLogic(doc.logic, doc.questions)
      } catch (error) {
        assert.ok(error instanceof LogicError)
        valid = false
      }
      assert.equal(valid, item.configuration_valid, `${path}：${item.name}`)
    }
  }
  const doc = copy(source)
  delete doc.logic
  doc.questions.length = 1
  doc.questions[0].options[0].id = contracts.canonical_id
  const question_id = doc.questions[0].id
  for (const item of contracts.uuid_cases) {
    const evaluate = () => new FlowSession(doc).evaluateAnswers([{ question_id, value: item.raw }])
    if (item.option_valid)
      assert.equal(evaluate().answers[question_id], contracts.canonical_id, item.name)
    else assert.throws(evaluate, LogicError, item.name)
  }
  doc.questions[0] = copy(source.questions[0])
  doc.questions[0].id = contracts.canonical_id
  const value = doc.questions[0].options[0].id
  for (const item of contracts.uuid_cases) {
    const answer = { question_id: item.raw, value }
    for (const run of [
      () => new FlowSession(doc).evaluateAnswers([answer]),
      () => new FlowSession(doc).dispatch({ type: 'change', ...answer }),
    ]) {
      if (item.configuration_valid) assert.doesNotThrow(run, item.name)
      else assert.throws(run, LogicError, item.name)
    }
  }
})

test('共享流程拒绝非法拓扑和未保护的分支依赖', () => {
  for (const fixture of fixtures.compilation) {
    const document = copy(fixtures.documents[fixture.document])
    for (const edit of fixture.edits) {
      const target = edit.path.slice(0, -1).reduce((target, key) => target[key], document)
      target[edit.path.at(-1)] = copy(edit.value)
    }
    let actual = null
    try {
      compileDocumentLogic(document.logic, document.questions)
    } catch (error) {
      assert.ok(error instanceof LogicError, fixture.name)
      actual = error.code
    }
    assert.equal(actual, fixture.error, fixture.name)
  }
})

test('共享事件序列保持路径清理回退和题型规范化一致', () => {
  for (const fixture of fixtures.sessions) {
    const session = new FlowSession(fixtures.documents[fixture.document])
    fixture.steps.forEach((step, index) => {
      const before = session.snapshot()
      if (step.event_json) {
        let actual = null
        try {
          session.dispatch(decode(step.event_json))
        } catch (error) {
          assert.ok(error instanceof LogicError, fixture.name)
          actual = error.code
        }
        assert.equal(actual, step.error, `${fixture.name}，第 ${index} 步`)
        if (actual) assert.deepEqual(session.snapshot(), before, '失败事件不能改变状态')
      }
      const actual = session.snapshot()
      for (const [key, value] of Object.entries(step.expected))
        assert.deepEqual(actual[key], value, `${fixture.name}，第 ${index} 步，${key}`)
    })
  }
})

test('整卷和单题复制重写流程引用且会话与外部修改隔离', () => {
  const source = copy(fixtures.documents.diamond)
  const before = copy(source)
  const duplicated = duplicateSurveyDocument(source)
  assert.deepEqual(source, before)
  assert.doesNotThrow(() => compileDocumentLogic(duplicated.logic, duplicated.questions))
  for (const node of source.logic.flow.nodes)
    assert.ok(!JSON.stringify(duplicated).includes(node.id), '复制不能遗留来源节点身份')
  const question = duplicated.questions[0]
  const newQuestion = duplicateQuestion(question, duplicated)
  duplicated.questions.splice(1, 0, newQuestion)
  assert.deepEqual(duplicated.logic.flow.nodes[0].question_ids, [question.id, newQuestion.id])
  assert.doesNotThrow(() => compileDocumentLogic(duplicated.logic, duplicated.questions))
  const session = new FlowSession(source)
  const snapshot = session.snapshot()
  source.questions.length = 0
  snapshot.history.length = 0
  assert.deepEqual(session.snapshot().history, [before.logic.flow.entry])
  assert.deepEqual(session.snapshot().visible, [before.questions[0].id])
})

test('最终提交共享样例拒绝路径外输入并重建适用性', () => {
  const submissions = JSON.parse(
    readFileSync(new URL('../../fixtures/survey-logic/submissions.json', import.meta.url), 'utf8'),
  )
  for (const fixture of submissions.cases) {
    const session = new FlowSession(fixtures.documents[fixture.document])
    const before = session.snapshot()
    let actual
    try {
      actual = { value: session.evaluateAnswers(decode(fixture.answers_json)) }
    } catch (error) {
      assert.ok(error instanceof LogicError, fixture.name)
      actual = { error: { code: error.code, question_id: error.question_id } }
    }
    assert.deepEqual(actual, fixture.expected, fixture.name)
    assert.deepEqual(session.snapshot(), before, '最终路径重建不能修改交互状态')
  }
})
