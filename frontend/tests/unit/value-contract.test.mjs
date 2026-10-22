//! 空白与文本合法性契约：与 Rust 的 Unicode 标量规则保持一致。
import assert from 'node:assert/strict'
import test from 'node:test'
import { isBlank, trimWhitespace, validText } from '../../src/value-contract.ts'

test('isBlank 识别 ASCII 与 Unicode 空白但不识别 BOM', () => {
  for (const value of ['', ' ', '\t\r\n', '\u00a0\u2003\u3000', '\u0085', '\u2028\u2029'])
    assert.equal(isBlank(value), true, `空白文本：${JSON.stringify(value)}`)
  for (const value of ['\ufeff', ' \ufeff ', 'a', '0', '\ufeff\u0085'])
    assert.equal(isBlank(value), false, `非空白文本：${JSON.stringify(value)}`)
})

test('trimWhitespace 只裁剪首尾空白并保留 BOM 与内部空白', () => {
  assert.equal(trimWhitespace(' \t\u0085文本\u3000 '), '文本')
  assert.equal(trimWhitespace('甲  乙'), '甲  乙')
  assert.equal(trimWhitespace('\ufeff'), '\ufeff')
  assert.equal(trimWhitespace(' \ufeff x \ufeff '), '\ufeff x \ufeff')
  assert.equal(trimWhitespace(''), '')
  const once = trimWhitespace('  x  ')
  assert.equal(trimWhitespace(once), once, '裁剪必须幂等')
})

test('validText 允许成对代理项并拒绝 NUL 与孤立代理项', () => {
  assert.equal(validText('普通文本'), true)
  assert.equal(validText('😀'), true, '成对代理项是合法 Unicode 标量')
  assert.equal(validText('\uD83D\uDE00'), true)
  assert.equal(validText('a\0b'), false)
  assert.equal(validText('\0'), false)
  assert.equal(validText('\uD800'), false, '孤立高位代理项不允许进入 Rust String')
  assert.equal(validText('\uDC00'), false, '孤立低位代理项不允许进入 Rust String')
  assert.equal(validText('a\uD800b'), false)
})
