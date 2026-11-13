//! 文本工作台辅助：wasm 巡检一次返回绑定与诊断，前端只负责可读名称与偏移换算。
import assert from 'node:assert/strict'
import test from 'node:test'
import { textBindingLabel, textErrorOffset } from '../../src/logic-text-tools.ts'
import { LogicTextError } from '../../src/logic.ts'
import { formatLogicText, inspectLogicText } from '../../src/wasm/index.ts'
import { readFlowDocument } from '../helpers/fixtures.mjs'

test('inspectLogicText 同时返回绑定与诊断，注释与字符串不参与扫描', () => {
  const document = readFlowDocument('diamond')
  const [question] = document.questions
  const variable = document.logic.variables[0]
  const node = document.logic.flow.nodes[0]
  const source = [
    `bind question q = "${question.id}"; // 行尾注释`,
    `bind option o = "${question.options[0].id}";`,
    `bind variable v = "${variable.id}";`,
    `bind node n = "${node.id}";`,
    'bind question gone = "00000000-0000-4000-8000-000000009999";',
    'bind option gone = "00000000-0000-4000-8000-000000009998";',
    '// bind question fake = "00000000-0000-4000-8000-000000000001";',
    '"bind question fake = \\"伪身份\\";"',
    'bind question 1bad = "00000000-0000-4000-8000-000000000001";',
    'let broken = ;',
  ].join('\n')
  const inspection = inspectLogicText(source, document.questions)
  assert.deepEqual(
    inspection.bindings.map((binding) => ({ ...binding })),
    [
      { category: 'question', alias: 'q', id: question.id },
      { category: 'option', alias: 'o', id: question.options[0].id },
      { category: 'variable', alias: 'v', id: variable.id },
      { category: 'node', alias: 'n', id: node.id },
      { category: 'question', alias: 'gone', id: '00000000-0000-4000-8000-000000009999' },
      { category: 'option', alias: 'gone', id: '00000000-0000-4000-8000-000000009998' },
    ],
  )
  // 语法错误保留在结果里，右侧速查栏不会因报错而清空。
  assert.ok(inspection.error instanceof LogicTextError)
  assert.equal(inspection.error.code, 'invalid_syntax')
  assert.equal(inspection.line, inspection.error.line)
  assert.equal(inspection.column, inspection.error.column)
})

test('合法文本不返回诊断，绑定覆盖四类身份并带可读名称', () => {
  const document = readFlowDocument('diamond')
  const source = formatLogicText(document.logic, document.questions)
  const inspection = inspectLogicText(source, document.questions)
  assert.equal(inspection.error, null)
  assert.ok(inspection.bindings.length >= 4)
  const categories = new Set(inspection.bindings.map((binding) => binding.category))
  assert.deepEqual([...categories].sort(), ['node', 'option', 'question', 'variable'])
  const questionBinding = inspection.bindings.find((binding) => binding.category === 'question')
  assert.ok(questionBinding)
  assert.equal(
    textBindingLabel(questionBinding, document.questions),
    document.questions.find((question) => question.id === questionBinding.id)?.title,
  )
  const missing = {
    category: 'question',
    alias: 'gone',
    id: '00000000-0000-4000-8000-000000009999',
  }
  assert.equal(textBindingLabel(missing, document.questions), '未命名或已删除题目')
})

test('textErrorOffset 按行起始与 Unicode 标量换算', () => {
  assert.equal(textErrorOffset('abc\ndef', 1, 1), 0)
  assert.equal(textErrorOffset('abc\ndef', 1, 2), 1)
  assert.equal(textErrorOffset('abc\ndef', 2, 1), 4)
  assert.equal(textErrorOffset('abc\ndef', 2, 3), 6)
  const crlf = 'a\r\nb\r\nc'
  assert.equal(textErrorOffset(crlf, 2, 1), 3)
  assert.equal(textErrorOffset(crlf, 3, 1), 6)
  const unicode = '// 😀\nvalidate q1 = "𠀀";'
  assert.equal(unicode.slice(textErrorOffset(unicode, 2, 18)), ';')
})
