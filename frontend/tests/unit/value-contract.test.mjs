//! 空白契约：与 Rust 的 Unicode 空白规则保持一致。
import assert from 'node:assert/strict'
import test from 'node:test'
import { isBlank, trimWhitespace } from '../../src/value-contract.ts'

// isBlank 与 trimWhitespace 遵循跨端空白规则
test('isBlank and trimWhitespace follow cross-platform whitespace rules', () => {
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
