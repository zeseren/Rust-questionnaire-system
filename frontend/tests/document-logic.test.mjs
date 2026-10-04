//! 文档依赖、复制和预算回归使用与 Rust 相同的规则样例。
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import { decode } from '../src/json.ts'
import { compileDocumentLogic, duplicateSurveyDocument } from '../src/document-logic.ts'
import { LogicError } from '../src/logic.ts'
import { duplicateQuestion } from '../src/documents.ts'

const expressions = JSON.parse(
  readFileSync(new URL('../../fixtures/survey-logic/expressions.json', import.meta.url), 'utf8'),
)
const documents = JSON.parse(
  readFileSync(new URL('../../fixtures/survey-logic/documents.json', import.meta.url), 'utf8'),
)

test('文档共享样例保持变量排序和间接依赖规则一致', () => {
  for (const fixture of documents.cases) {
    let actual
    try {
      const compiled = compileDocumentLogic(fixture.logic, expressions.questions)
      actual = {
        order: compiled.variableOrder,
        value: compiled.evaluate(decode(fixture.answers_json)),
      }
    } catch (error) {
      assert.ok(error instanceof LogicError, fixture.name)
      actual = { error: { code: error.code, question_id: error.question_id } }
    }
    assert.deepEqual(actual, fixture.expected, fixture.name)
  }
})

test('整卷复制重写题目选项变量身份并保留旧文档格式', () => {
  const old = {
    title: '复制回归',
    description: '',
    purpose: '回归',
    thank_you: '',
    questions: expressions.questions,
  }
  assert.equal(Object.hasOwn(duplicateSurveyDocument(old), 'logic'), false)
  const source = { ...old, logic: documents.cases[0].logic }
  const before = JSON.stringify(source)
  const copy = duplicateSurveyDocument(source)
  assert.equal(JSON.stringify(source), before)
  const copied = JSON.stringify(copy)
  const ids = [
    ...source.questions.flatMap((question) => [
      question.id,
      ...(question.options ?? []).map((option) => option.id),
    ]),
    ...source.logic.variables.map((variable) => variable.id),
  ]
  for (const id of ids) assert.ok(!copied.includes(id), `复制不能遗留来源身份：${id}`)
  assert.doesNotThrow(() => compileDocumentLogic(copy.logic, copy.questions))
})

test('文档总预算不能被拆成多个合法小表达式绕过', () => {
  const expression = {
    op: 'all',
    expressions: Array.from({ length: 255 }, () => ({ op: 'boolean', value: true })),
  }
  const variables = Array.from({ length: 17 }, (_, index) => ({
    id: `00000000-0000-0000-0000-${(index + 1).toString(16).padStart(12, '0')}`,
    value_type: 'boolean',
    expression,
  }))
  const logic = { dsl_version: 1, semantics_version: 1, variables, rules: [] }
  assert.throws(() => compileDocumentLogic(logic, []), { code: 'limit_exceeded' })
  variables.pop()
  assert.doesNotThrow(() => compileDocumentLogic(logic, []))
})

test('单题复制保留上游规则并重写当前题约束', () => {
  const document = {
    questions: structuredClone(expressions.questions),
    logic: structuredClone(documents.cases[0].logic),
  }
  const source = document.questions.at(-1)
  const copied = duplicateQuestion(source, document)
  document.questions.push(copied)
  const rule = document.logic.rules.at(-1)
  assert.equal(rule.question_id, copied.id)
  assert.deepEqual(rule.visible_if, document.logic.rules[0].visible_if)
  assert.equal(rule.validate.left.question_id, copied.id)
  assert.equal(document.logic.rules[0].validate.left.question_id, source.id)
  assert.doesNotThrow(() => compileDocumentLogic(document.logic, document.questions))
})
