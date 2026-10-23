//! 空白与文本合法性契约：与 Rust 的 Unicode 标量规则保持一致。
import assert from 'node:assert/strict'
import test from 'node:test'
import { isBlank, trimWhitespace, validText } from '../../src/value-contract.ts'

test('isBlank 与 trimWhitespace 遵循跨端空白规则', () => {
  for (const [value, blank, trimmed] of [
    ['', true, ''],
    ['\u0085', true, ''],
    ['\u3000', true, ''],
    ['\ufeff', false, '\ufeff'],
    [' \ufeff ', false, '\ufeff'],
    ['甲  乙', false, '甲  乙'],
  ]) {
    assert.equal(isBlank(value), blank, `isBlank：${JSON.stringify(value)}`)
    assert.equal(trimWhitespace(value), trimmed, `trimWhitespace：${JSON.stringify(value)}`)
  }
})

test('validText 允许成对代理项并拒绝 NUL 与孤立代理项', () => {
  assert.equal(validText('普通文本'), true)
  assert.equal(validText('😀'), true, '成对代理项是合法 Unicode 标量')
  assert.equal(validText('a\0b'), false)
  assert.equal(validText('\0'), false)
  assert.equal(validText('\uD800'), false, '孤立高位代理项不允许进入 Rust String')
  assert.equal(validText('\uDC00'), false, '孤立低位代理项不允许进入 Rust String')
  assert.equal(validText('a\uD800b'), false)
})
