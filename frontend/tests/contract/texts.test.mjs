//! 文本 DSL 契约：解析 AST、错误行列与逐字节格式化输出两端一致。
import assert from 'node:assert/strict'
import test from 'node:test'
import { LogicTextError } from '../../src/logic.ts'
import { formatLogicText, parseLogicText } from '../../src/wasm/index.ts'
import { applyEdits, loadFixture, readFlowDocument } from '../helpers/fixtures.mjs'

const fixtures = loadFixture('texts')
const flows = loadFixture('flows').documents
const document = readFlowDocument(fixtures.document)

/** 格式样例字段：显式值优先，否则按 document 名读取共享流程文档；无效引用必须报错。 */
function formatCaseField(item, field) {
  if (item[field] !== undefined) return item[field]
  if (item.document === undefined) throw new Error(`${item.name}：缺少 ${field} 或 document 引用`)
  const source = flows[item.document]
  if (!source) throw new Error(`${item.name}：未知流程文档 ${item.document}`)
  if (source[field] === undefined) {
    throw new Error(`${item.name}：流程文档 ${item.document} 缺少 ${field}`)
  }
  return source[field]
}

/** 格式样例规则：logic_overrides 覆盖命名文档的 logic，缺省时改用 format_base_logic。 */
function formatCaseLogic(item) {
  if (item.logic_overrides === undefined) return formatCaseField(item, 'logic')
  const source = flows[item.document]
  if (!source) throw new Error(`${item.name}：未知流程文档 ${item.document}`)
  const base = source.logic ?? fixtures.format_base_logic
  if (!base) throw new Error(`${item.name}：缺少 logic 基底`)
  return { ...base, ...item.logic_overrides }
}

/** 共享样例用替换构造单处错误，因此错误不依赖两端文案。 */
function buildSource(item) {
  let source = fixtures.source_lines.join('\n')
  for (const [from, to] of item.replacements) source = source.replaceAll(from, to)
  return source + (item.append ?? '')
}

// 共享文本样例保持身份优先级、解析结果与错误行列一致
test('shared text fixtures keep identity precedence, parse results and error positions consistent', () => {
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
    // 别名提示只影响文本展示，不改变身份与表达式结构；与 Rust 契约测试一致，单独校验 AST。
    const { aliases: _aliases, ...actual } = parseLogicText(source, document.questions)
    delete expected.aliases
    assert.deepEqual(actual, expected, item.name)
  }
})

// 共享格式化样例逐字节输出、二次解析等值且二次格式化幂等
test('shared formatting fixtures produce byte-exact output, equal reparse and idempotent reformat', () => {
  for (const item of fixtures.format_cases) {
    const questions = formatCaseField(item, 'questions')
    const logic = formatCaseLogic(item)
    const text = formatLogicText(logic, questions)
    assert.equal(text, item.expected, `${item.name}：逐字节输出`)
    const reparsed = parseLogicText(text, questions)
    assert.deepEqual(reparsed, logic, `${item.name}：二次解析等值`)
    assert.equal(formatLogicText(reparsed, questions), text, `${item.name}：二次格式化幂等`)
  }
})

// 全部共享流程文档的规则可以格式化后无损还原
test('all shared flow document rules survive formatting losslessly', () => {
  for (const [name, entry] of Object.entries(loadFixture('flows').documents)) {
    if (!entry.logic) continue
    const text = formatLogicText(entry.logic, entry.questions)
    assert.deepEqual(parseLogicText(text, entry.questions), entry.logic, name)
  }
})
