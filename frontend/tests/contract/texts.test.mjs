//! 文本 DSL 契约：解析 AST、错误行列与逐字节格式化输出两端一致。
import assert from 'node:assert/strict'
import test from 'node:test'
import { LogicTextError } from '../../src/logic.ts'
import { formatLogicText, parseLogicText } from '../../src/wasm/index.ts'
import { applyEdits, loadFixture, readFlowDocument } from '../helpers/fixtures.mjs'

const fixtures = loadFixture('texts')
const document = readFlowDocument(fixtures.document)

/** 共享样例用替换构造单处错误，因此错误不依赖两端文案。 */
function buildSource(item) {
  let source = fixtures.source_lines.join('\n')
  for (const [from, to] of item.replacements) source = source.replaceAll(from, to)
  return source + (item.append ?? '')
}

test('共享文本样例保持身份优先级、解析结果与错误行列一致', () => {
  for (const item of fixtures.cases) {
    const source = buildSource(item)
    if (item.error) {
      assert.throws(
        () => parseLogicText(source, document.questions),
        (error) =>
          error instanceof LogicTextError &&
          error.code === item.error &&
          error.line === item.line &&
          error.column === item.column,
        item.name,
      )
      continue
    }
    const expected = applyEdits(structuredClone(document.logic), item.edits)
    assert.deepEqual(parseLogicText(source, document.questions), expected, item.name)
  }
})

test('共享格式化样例逐字节输出、二次解析等值且二次格式化幂等', () => {
  for (const item of fixtures.format_cases) {
    const text = formatLogicText(item.logic, item.questions)
    assert.equal(text, item.expected, `${item.name}：逐字节输出`)
    const reparsed = parseLogicText(text, item.questions)
    assert.deepEqual(reparsed, item.logic, `${item.name}：二次解析等值`)
    assert.equal(formatLogicText(reparsed, item.questions), text, `${item.name}：二次格式化幂等`)
  }
})

test('全部共享流程文档的规则可以格式化后无损还原', () => {
  for (const [name, entry] of Object.entries(loadFixture('flows').documents)) {
    if (!entry.logic) continue
    const text = formatLogicText(entry.logic, entry.questions)
    assert.deepEqual(parseLogicText(text, entry.questions), entry.logic, name)
  }
})
