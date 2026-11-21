//! 数字输入边界：修剪空白后只接受可精确表示有限双精度的十进制写法。
import assert from 'node:assert/strict'
import test from 'node:test'
import { isNumberLiteral, isNumberPrefix } from '../../src/numbers.ts'

// isNumberLiteral 接受整数、小数、科学计数法与负零
test('isNumberLiteral accepts integers, decimals, scientific notation and negative zero', () => {
  for (const raw of ['0', '-0', '12', '-12', '0.5', '1e3', '1E+3', '1e-3', '1e308'])
    assert.equal(isNumberLiteral(raw), true, raw)
})

// isNumberLiteral 拒绝非有限值与不规范写法
test('isNumberLiteral rejects non-finite values and malformed notation', () => {
  for (const raw of [
    '',
    ' 12',
    '+1',
    '01',
    '00',
    '.5',
    '1.',
    '1e',
    'Infinity',
    '-Infinity',
    'NaN',
    '1e400',
    '0x10',
    '1,2',
    '--1',
    '−1',
  ])
    assert.equal(isNumberLiteral(raw), false, JSON.stringify(raw))
})

// isNumberPrefix 只认还能补全成合法数字的未完成输入
test('isNumberPrefix accepts only incomplete input that can still complete to a valid number', () => {
  for (const raw of ['-', '0.', '12.', '-12.', '1e', '1E+', '1e-', '1.5e'])
    assert.equal(isNumberPrefix(raw), true, JSON.stringify(raw))
  // 完整数字、空串与永远补不成的写法都不算输入中。
  for (const raw of ['', '0', '-0', '12.5', '1e3', '.', '.5', '-.', '00', '01', '1.e', '+', 'abc'])
    assert.equal(isNumberPrefix(raw), false, JSON.stringify(raw))
})
