//! 数字输入边界：修剪空白后只接受可精确表示有限双精度的十进制写法。
import assert from 'node:assert/strict'
import test from 'node:test'
import { isNumberPrefix, numberError } from '../../src/numbers.ts'

const message = '请输入有限数字，例如 12、0.5 或 1e3。'

test('numberError 接受整数、小数、科学计数法与负零', () => {
  for (const raw of ['0', '-0', '12', '-12', '0.5', '1e3', '1E+3', '1e-3', '1e308'])
    assert.equal(numberError(raw), undefined, raw)
})

test('numberError 拒绝非有限值与不规范写法并给出固定文案', () => {
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
    assert.equal(numberError(raw), message, JSON.stringify(raw))
})

test('isNumberPrefix 只认还能补全成合法数字的未完成输入', () => {
  for (const raw of ['-', '0.', '12.', '-12.', '1e', '1E+', '1e-', '1.5e'])
    assert.equal(isNumberPrefix(raw), true, JSON.stringify(raw))
  // 完整数字、空串与永远补不成的写法都不算输入中。
  for (const raw of ['', '0', '-0', '12.5', '1e3', '.', '.5', '-.', '00', '01', '1.e', '+', 'abc'])
    assert.equal(isNumberPrefix(raw), false, JSON.stringify(raw))
})
