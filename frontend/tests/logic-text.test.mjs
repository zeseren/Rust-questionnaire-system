//! 两端解析同一文本得到同一 AST；错误文本不能绕过已有语义与容量检查。
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import { copy } from '../src/json.ts'
import { LogicTextError } from '../src/logic.ts'
import { compileExpression, formatLogicText, parseLogicText } from '../src/wasm/index.ts'
const read = (name) =>
  JSON.parse(
    readFileSync(new URL(`../../fixtures/survey-logic/${name}.json`, import.meta.url), 'utf8'),
  )
const fixture = read('texts'),
  documents = read('flows').documents

test('文本共享样例保留身份优先级并定位语法与语义错误', () => {
  for (const item of fixture.cases) {
    let source = fixture.source_lines.join('\n')
    for (const [from, to] of item.replacements) source = source.replaceAll(from, to)
    source += item.append ?? ''
    const document = documents[fixture.document]
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
    } else {
      const expected = copy(document.logic)
      for (const edit of item.edits)
        edit.path.slice(0, -1).reduce((value, key) => value[key], expected)[edit.path.at(-1)] =
          edit.value
      const result = parseLogicText(source, document.questions)
      assert.deepEqual(result, expected, item.name)
    }
  }
})

test('有效表达式和流程可格式化并二次解析往返', () => {
  const expressions = read('expressions')
  // 使用人工预期不含错误的样例；编译意外失败时直接报告，不静默跳过。
  for (const item of expressions.cases.filter((item) => !item.expected.error)) {
    const compiled = compileExpression(item.program, expressions.questions)
    const logic = {
      dsl_version: 1,
      semantics_version: 1,
      variables: [
        {
          id: '00000000-0000-4000-8000-000000002001',
          value_type: compiled.valueType,
          expression: item.program.expression,
        },
      ],
      rules: [],
    }
    assert.deepEqual(
      parseLogicText(formatLogicText(logic, expressions.questions), expressions.questions),
      logic,
      item.name,
    )
  }
  for (const document of Object.values(documents))
    if (document.logic)
      assert.deepEqual(
        parseLogicText(formatLogicText(document.logic, document.questions), document.questions),
        document.logic,
      )
})
