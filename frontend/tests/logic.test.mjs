//! 与 Rust 使用相同的语义输入和人工期望值，防止预览与服务端求值不一致。
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import { parse } from 'lossless-json'
import { compileExpression, LogicError } from '../src/logic.ts'

const fixtures = JSON.parse(
  readFileSync(new URL('../../fixtures/survey-logic/expressions.json', import.meta.url), 'utf8'),
)

test('两端共享样例保持编译与求值语义一致', () => {
  for (const fixture of fixtures.cases) {
    let actual
    try {
      const compiled = compileExpression(fixture.program, fixtures.questions)
      actual = { value: compiled.evaluate(parse(fixture.answers_json)) }
    } catch (error) {
      assert.ok(error instanceof LogicError, `必须返回领域错误：${fixture.name}`)
      actual = { error: { code: error.code, path: error.path, question_id: error.question_id } }
    }
    assert.deepEqual(actual, fixture.expected, `语义样例：${fixture.name}`)
  }
})

test('深度和节点上限在编译之前拦截且允许边界', () => {
  for (const [depth, count, allowed] of [
    [16, 1, true],
    [17, 1, false],
    [2, 256, true],
    [2, 257, false],
  ]) {
    let expression = { op: 'boolean', value: true }
    if (count > 1)
      expression = { op: 'all', expressions: Array.from({ length: count - 1 }, () => expression) }
    else for (let i = 1; i < depth; i++) expression = { op: 'not', expression }
    const program = { dsl_version: 1, semantics_version: 1, expression }
    if (allowed) assert.doesNotThrow(() => compileExpression(program, []))
    else assert.throws(() => compileExpression(program, []), { code: 'limit_exceeded' })
  }
})

test('编译结果与原始配置隔离并保留去重依赖', () => {
  const questions = structuredClone(fixtures.questions)
  const id = questions[0].id
  const option = questions[0].options[0].id
  const program = {
    dsl_version: 1,
    semantics_version: 1,
    expression: {
      op: 'all',
      expressions: [
        { op: 'answered', question_id: id },
        { op: 'selected', question_id: id, option_id: option },
      ],
    },
  }
  const compiled = compileExpression(program, questions)
  program.expression.expressions.length = 0
  questions[0].options.length = 0
  assert.equal(compiled.valueType, 'boolean')
  assert.deepEqual(compiled.dependencies, [id])
  assert.deepEqual(compiled.evaluate({ [id]: option }), { type: 'boolean', value: true })
  assert.deepEqual(compiled.evaluate({}), { type: 'boolean', value: false })
})
