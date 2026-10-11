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

test('有效表达式和流程可格式化往返且解析预算生效', () => {
  const expressions = read('expressions')
  // 使用人工预期为有效值的样例；编译意外失败时直接报告，不静默跳过。
  for (const item of expressions.cases.filter((item) => item.expected.value)) {
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
  const source =
    'dsl 1 semantics 1;\nbind variable v = "00000000-0000-4000-8000-000000002001";\nlet v: number = '
  for (const expression of ['1 + '.repeat(20000) + '1', '('.repeat(65) + '1' + ')'.repeat(65)])
    assert.throws(
      () => parseLogicText(source + expression + ';', expressions.questions),
      (error) => error instanceof LogicTextError && error.code === 'limit_exceeded',
    )
  assert.throws(
    () => parseLogicText(' '.repeat(1048577), []),
    (error) => error instanceof LogicTextError && error.code === 'limit_exceeded',
  )
})

test('共享 format_cases 逐字节锁定格式化输出并可二次解析往返', () => {
  for (const item of fixture.format_cases) {
    // expected 由 TS formatLogicText 真实输出粘贴；双端必须逐字节一致。
    assert.equal(formatLogicText(item.logic, item.questions), item.expected, item.name)
    const once = parseLogicText(item.expected, item.questions)
    // 一次与二次解析必须等值，保证输出无歧义。
    assert.deepEqual(parseLogicText(formatLogicText(once, item.questions), item.questions), once, item.name)
  }
})
