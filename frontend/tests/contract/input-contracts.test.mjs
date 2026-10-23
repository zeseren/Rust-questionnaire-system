//! 输入契约：共享身份与空白样例在配置编译、答案归一和填写会话中一致。
import assert from 'node:assert/strict'
import test from 'node:test'
import { LogicError } from '../../src/logic.ts'
import { FlowSession, compileDocumentLogic } from '../../src/wasm/index.ts'
import {
  captureDomainError,
  domainErrorFields,
  loadFixture,
  readFlowDocument,
} from '../helpers/fixtures.mjs'
import { surveyDocument, textareaQuestion } from '../helpers/documents.mjs'

const contracts = loadFixture('input-contracts')

test('配置身份的每个引用位置只接受规范小写 UUID', () => {
  const source = readFlowDocument(contracts.document)
  for (const path of contracts.configuration_paths) {
    const segments = path.slice(1).split('/')
    const original = segments.reduce((value, key) => value[key], source)
    // 先同步重命名声明与引用，再只扰动一个字段，避免其他字段掩盖校验缺口。
    const renamed = JSON.stringify(source).replaceAll(original, contracts.canonical_id)
    for (const item of contracts.uuid_cases) {
      const document = JSON.parse(renamed)
      segments.slice(0, -1).reduce((value, key) => value[key], document)[segments.at(-1)] = item.raw
      const error = captureDomainError(() =>
        compileDocumentLogic(document.logic, document.questions),
      )
      assert.equal(error === null, item.configuration_valid, `${path}：${item.name}`)
    }
  }
})

test('选项答案只接受规范 UUID，大写与零值一律拒绝', () => {
  const document = readFlowDocument(contracts.document)
  delete document.logic
  document.questions.length = 1
  document.questions[0].options[0].id = contracts.canonical_id
  const question_id = document.questions[0].id
  for (const item of contracts.uuid_cases) {
    const evaluate = () =>
      new FlowSession(document).evaluateAnswers([{ question_id, value: item.raw }])
    if (item.option_valid)
      assert.equal(evaluate().answers[question_id], contracts.canonical_id, item.name)
    else assert.throws(evaluate, LogicError, item.name)
  }
})

test('题目身份在重建与交互事件中只接受规范且已声明的 UUID', () => {
  const source = readFlowDocument(contracts.document)
  const document = readFlowDocument(contracts.document)
  delete document.logic
  document.questions.length = 1
  document.questions[0] = structuredClone(source.questions[0])
  document.questions[0].id = contracts.canonical_id
  const value = document.questions[0].options[0].id
  for (const item of contracts.uuid_cases) {
    const answer = { question_id: item.raw, value }
    if (item.configuration_valid) {
      assert.doesNotThrow(() => new FlowSession(document).evaluateAnswers([answer]), item.name)
      assert.doesNotThrow(
        () => new FlowSession(document).dispatch({ type: 'change', ...answer }),
        item.name,
      )
      continue
    }
    // 大写、无连字符与零值身份都在 wasm 边界反序列化时失败。
    const boundary = { code: 'invalid_syntax', path: '$', question_id: null }
    assert.deepEqual(
      domainErrorFields(
        captureDomainError(() => new FlowSession(document).evaluateAnswers([answer])),
      ),
      boundary,
      item.name,
    )
    assert.deepEqual(
      domainErrorFields(
        captureDomainError(() => new FlowSession(document).dispatch({ type: 'change', ...answer })),
      ),
      boundary,
      item.name,
    )
  }
  // 规范但未声明的身份通过边界后，由领域层分别拒绝重建与事件。
  const unknown = { question_id: '00000000-0000-4000-8000-000000009999', value }
  assert.deepEqual(
    domainErrorFields(
      captureDomainError(() => new FlowSession(document).evaluateAnswers([unknown])),
    ),
    { code: 'invalid_reference', path: '$.answers', question_id: unknown.question_id },
  )
  assert.deepEqual(
    domainErrorFields(
      captureDomainError(() => new FlowSession(document).dispatch({ type: 'change', ...unknown })),
    ),
    { code: 'invalid_event', path: '$.event', question_id: null },
  )
})

test('共享文本样例在引擎答案归一上保持一致', () => {
  for (const item of contracts.text_cases) {
    for (const required of [false, true]) {
      const question = textareaQuestion({ required })
      const document = surveyDocument([question])
      const session = new FlowSession(document)
      session.dispatch({ type: 'change', question_id: question.id, value: item.value })
      const snapshot = session.snapshot()
      assert.equal(
        Object.hasOwn(snapshot.answers, question.id),
        item.answered,
        `${item.name}：必填=${required}`,
      )
      assert.equal(
        Boolean(snapshot.errors[question.id]),
        required && !item.answered,
        `${item.name}：必填=${required}`,
      )
      if (item.answered) assert.equal(snapshot.answers[question.id], item.value, item.name)
    }
  }
})
