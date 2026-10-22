//! 流程契约：共享拓扑错误与交互事件序列必须与引擎逐字段一致。
import assert from 'node:assert/strict'
import test from 'node:test'
import { decode } from '../../src/json.ts'
import { LogicError } from '../../src/logic.ts'
import { FlowSession, compileDocumentLogic } from '../../src/wasm/index.ts'
import { applyEdits, loadFixture, readFlowDocument } from '../helpers/fixtures.mjs'

const fixtures = loadFixture('flows')

test('共享流程编译样例保持拓扑错误码一致', () => {
  for (const item of fixtures.compilation) {
    const document = applyEdits(readFlowDocument(item.document), item.edits)
    let code = null
    try {
      compileDocumentLogic(document.logic, document.questions)
    } catch (error) {
      assert.ok(error instanceof LogicError, item.name)
      code = error.code
    }
    assert.equal(code, item.error, item.name)
  }
})

test('共享事件序列保持路径清理、回退与三态一致', () => {
  for (const item of fixtures.sessions) {
    const session = new FlowSession(readFlowDocument(item.document))
    item.steps.forEach((step, index) => {
      const label = `${item.name}，第 ${index} 步`
      const before = session.snapshot()
      if (step.event_json) {
        let code = null
        try {
          session.dispatch(decode(step.event_json))
        } catch (error) {
          assert.ok(error instanceof LogicError, label)
          code = error.code
        }
        assert.equal(code, step.error, label)
        if (code) assert.deepEqual(session.snapshot(), before, `${label}：失败事件不得改变状态`)
      }
      const actual = session.snapshot()
      for (const [key, value] of Object.entries(step.expected))
        assert.deepEqual(actual[key], value, `${label}：${key}`)
    })
  }
})
