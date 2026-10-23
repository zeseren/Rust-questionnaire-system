//! 文本编辑辅助：签名、引用提示与行列到 UTF-16 偏移换算。
import assert from 'node:assert/strict'
import test from 'node:test'
import { encode } from '../../src/json.ts'
import { logicTextSignature, textBindings, textErrorOffset } from '../../src/logic-text-tools.ts'
import { readFlowDocument } from '../helpers/fixtures.mjs'
import { createSurveyTemplate } from '../../src/survey-templates.ts'

test('logicTextSignature 使用规范化文本并对字段顺序与空值稳定', () => {
  const document = createSurveyTemplate('feedback')
  const original = logicTextSignature(document.logic, document.questions)
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
  assert.equal(logicTextSignature(document.logic, document.questions), encode(document.logic))
})

test('textBindings 识别四类绑定并忽略注释、字符串与非法别名', () => {
  const document = readFlowDocument('diamond')
  assert.deepEqual(textBindings('', document.questions), [])
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
