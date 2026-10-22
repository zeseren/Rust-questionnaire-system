//! 提交契约：最终答案重建路径、三态与拒绝码必须与引擎一致。
import assert from 'node:assert/strict'
import test from 'node:test'
import { decode } from '../../src/json.ts'
import { LogicError } from '../../src/logic.ts'
import { FlowSession } from '../../src/wasm/index.ts'
import { domainErrorFields, loadFixture, readFlowDocument } from '../helpers/fixtures.mjs'

const fixtures = loadFixture('submissions')

test('共享提交样例保持答案、适用性、路径与拒绝码一致', () => {
  for (const item of fixtures.cases) {
    const session = new FlowSession(readFlowDocument(item.document))
    const before = session.snapshot()
    let actual
    try {
      actual = { value: session.evaluateAnswers(decode(item.answers_json)) }
    } catch (error) {
      if (!(error instanceof LogicError)) throw error
      assert.ok(error.path === '$.answers' || error.path === '$.route', item.name)
      actual = { error: domainErrorFields(error, item.expected.error) }
    }
    assert.deepEqual(actual, item.expected, item.name)
    assert.deepEqual(session.snapshot(), before, `${item.name}：路径重建不得修改交互状态`)
  }
})

test('连续重建结果一致且快照保持只读', () => {
  const item = fixtures.cases.find((entry) => entry.expected.value)
  assert.ok(item, '共享样例必须包含成功重建的用例')
  const session = new FlowSession(readFlowDocument(item.document))
  const answers = decode(item.answers_json)
  const first = session.evaluateAnswers(answers)
  first.answers = {}
  first.question_states = {}
  const second = session.evaluateAnswers(answers)
  assert.deepEqual(second, item.expected.value, item.name)
})
