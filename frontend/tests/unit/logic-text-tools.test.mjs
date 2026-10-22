//! 文本编辑辅助：签名、引用提示与行列到 UTF-16 偏移换算。
import assert from 'node:assert/strict'
import test from 'node:test'
import { encode } from '../../src/json.ts'
import { LogicError } from '../../src/logic.ts'
import { logicTextSignature, textBindings, textErrorOffset } from '../../src/logic-text-tools.ts'
import { formatLogicText } from '../../src/wasm/index.ts'
import { readFlowDocument } from '../helpers/fixtures.mjs'
import { createSurveyTemplate } from '../../src/survey-templates.ts'

test('logicTextSignature 使用规范化文本并对字段顺序与空值稳定', () => {
  const document = createSurveyTemplate('feedback')
  const original = logicTextSignature(document.logic, document.questions)
  assert.equal(original, formatLogicText(document.logic, document.questions))
  const response = {
    flow: null,
    rules: document.logic.rules.map((rule) => ({ validate: null, ...rule })),
    variables: document.logic.variables.map((variable) => ({
      expression: variable.expression,
      value_type: variable.value_type,
      id: variable.id,
    })),
    semantics_version: 1,
    dsl_version: 1,
  }
  assert.equal(logicTextSignature(response, document.questions), original)
  response.variables[0].expression.right.value = '2'
  assert.notEqual(
    logicTextSignature(response, document.questions),
    original,
    '实际规则修改仍需提示同步',
  )
})

test('格式化失败时签名回退为原始 JSON', () => {
  const document = readFlowDocument('diamond')
  document.logic.dsl_version = 2
  assert.throws(
    () => formatLogicText(document.logic, document.questions),
    (error) => error instanceof LogicError && error.code === 'unsupported_version',
  )
  assert.equal(logicTextSignature(document.logic, document.questions), encode(document.logic))
})

test('textBindings 识别四类绑定并给出友好标签', () => {
  const document = readFlowDocument('diamond')
  const [question] = document.questions
  const variable = document.logic.variables[0]
  const node = document.logic.flow.nodes[0]
  const source = [
    `bind question q = "${question.id}";`,
    `bind option o = "${question.options[0].id}";`,
    `bind variable v = "${variable.id}";`,
    `bind node n = "${node.id}";`,
    'bind question gone = "00000000-0000-4000-8000-000000009999";',
    'bind option gone = "00000000-0000-4000-8000-000000009998";',
    '// bind question fake = "00000000-0000-4000-8000-000000000001";',
    '"bind question fake = \\"伪身份\\";"',
    'bind question 1bad = "00000000-0000-4000-8000-000000000001";',
  ].join('\n')
  assert.deepEqual(textBindings(source, document.questions), [
    { category: 'question', alias: 'q', id: question.id, label: question.title },
    {
      category: 'option',
      alias: 'o',
      id: question.options[0].id,
      label: question.options[0].label,
    },
    { category: 'variable', alias: 'v', id: variable.id, label: '计算变量' },
    { category: 'node', alias: 'n', id: node.id, label: '流程节点' },
    {
      category: 'question',
      alias: 'gone',
      id: '00000000-0000-4000-8000-000000009999',
      label: '未命名或已删除题目',
    },
    {
      category: 'option',
      alias: 'gone',
      id: '00000000-0000-4000-8000-000000009998',
      label: '未命名或已删除选项',
    },
  ])
})

test('textBindings 不把注释、字符串与非法别名当成声明', () => {
  const document = readFlowDocument('diamond')
  assert.deepEqual(textBindings('', document.questions), [])
  const source = [
    '// bind question fake = "00000000-0000-4000-8000-000000000001";',
    '"bind question fake = \\"伪身份\\";"',
    'bind question 1bad = "00000000-0000-4000-8000-000000000001";',
    `bind question ok = "${document.questions[0].id}"; // 行尾注释`,
  ].join('\n')
  const bindings = textBindings(source, document.questions)
  assert.equal(bindings.length, 1)
  assert.equal(bindings[0].alias, 'ok')
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

test('越界行列按切片规则返回确定的偏移', () => {
  const source = '// 😀\nvalidate q1 = "𠀀";'
  for (const [line, column, expected] of [
    // 行号小于 1：累计除最后一行外的全部长度。
    [0, 1, 6],
    // 列号 0：`slice(0, -1)` 少一个码点。
    [1, 0, 3],
    // 超出首行：取整行长度。
    [1, 99, 5],
    // 第二行首列少最后一个码点，超出行长取整行。
    [2, 0, 24],
    [2, 99, 25],
    // 行号越界：累计全部行并额外计一个换行。
    [99, 1, 26],
  ])
    assert.equal(textErrorOffset(source, line, column), expected, `${line}:${column}`)
})
