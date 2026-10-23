//! 文档逻辑契约：变量拓扑顺序、规则求值与共享错误定位。
import assert from 'node:assert/strict'
import test from 'node:test'
import { decode } from '../../src/json.ts'
import { LogicError } from '../../src/logic.ts'
import { compileDocumentLogic } from '../../src/wasm/index.ts'
import { domainErrorFields, loadFixture } from '../helpers/fixtures.mjs'

const expressions = loadFixture('expressions')
const fixtures = loadFixture('documents')

test('共享文档样例保持变量顺序、规则结果与错误定位一致', () => {
  for (const item of fixtures.cases) {
    let actual
    try {
      const compiled = compileDocumentLogic(item.logic, expressions.questions)
      actual = {
        order: compiled.variableOrder,
        value: compiled.evaluate(decode(item.answers_json)),
      }
    } catch (error) {
      if (!(error instanceof LogicError)) throw error
      actual = { error: domainErrorFields(error, item.expected.error) }
    }
    assert.deepEqual(actual, item.expected, item.name)
  }
})
