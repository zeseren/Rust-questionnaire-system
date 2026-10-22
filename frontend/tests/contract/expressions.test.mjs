//! 表达式契约：前端包装层与 Rust 引擎消费同一份共享样例。
import assert from 'node:assert/strict'
import test from 'node:test'
import { decode } from '../../src/json.ts'
import { LogicError } from '../../src/logic.ts'
import { compileExpression } from '../../src/wasm/index.ts'
import { captureDomainError, domainErrorFields, loadFixture } from '../helpers/fixtures.mjs'

const fixture = loadFixture('expressions')

/** 递归收集表达式引用的题目身份；编译依赖必须是它的精确集合。 */
function referencedQuestionIds(raw) {
  const ids = new Set()
  const walk = (node) => {
    if (Array.isArray(node)) {
      node.forEach(walk)
      return
    }
    if (typeof node !== 'object' || node === null) return
    if (typeof node.question_id === 'string') ids.add(node.question_id)
    for (const value of Object.values(node)) walk(value)
  }
  walk(raw)
  return [...ids].sort()
}

test('共享表达式样例保持编译类型、依赖与求值结果一致', () => {
  for (const item of fixture.cases) {
    let actual
    try {
      const compiled = compileExpression(item.program, fixture.questions)
      assert.deepEqual(
        [...compiled.dependencies].sort(),
        referencedQuestionIds(item.program),
        `${item.name}：依赖集合`,
      )
      if (item.expected.value && item.expected.value.type !== 'missing')
        assert.equal(compiled.valueType, item.expected.value.type, `${item.name}：静态类型`)
      actual = { value: compiled.evaluate(decode(item.answers_json)) }
    } catch (error) {
      if (!(error instanceof LogicError)) throw error
      actual = { error: domainErrorFields(error) }
    }
    assert.deepEqual(actual, item.expected, item.name)
  }
})

test('缺失值不产生依赖之外的题目引用', () => {
  const missing = fixture.cases.filter((item) => item.expected.value?.type === 'missing')
  assert.ok(missing.length > 0, '共享样例必须覆盖三值逻辑的未知结果')
  for (const item of missing) {
    const compiled = compileExpression(item.program, fixture.questions)
    assert.deepEqual([...compiled.dependencies].sort(), referencedQuestionIds(item.program))
  }
})

test('非法表达式在编译或求值时返回带路径的领域错误', () => {
  const errors = fixture.cases.filter((item) => item.expected.error)
  assert.ok(errors.length > 0, '共享样例必须覆盖错误结果')
  for (const item of errors) {
    const error = captureDomainError(() => {
      const compiled = compileExpression(item.program, fixture.questions)
      compiled.evaluate(decode(item.answers_json))
    })
    assert.ok(error instanceof LogicError, item.name)
    assert.equal(error.code, item.expected.error.code, `${item.name}：错误码`)
    assert.equal(error.path, item.expected.error.path, `${item.name}：错误路径`)
    assert.equal(error.question_id, item.expected.error.question_id, `${item.name}：关联题目`)
  }
})
