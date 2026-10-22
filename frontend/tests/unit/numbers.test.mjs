//! 数字输入边界：修剪空白后只接受可精确表示有限双精度的十进制写法。
import assert from 'node:assert/strict'
import test from 'node:test'
import { numberError, numberInput } from '../../src/numbers.ts'

const message = '请输入有限数字，例如 12、0.5 或 1e3。'

test('numberInput 仅裁剪首尾空白', () => {
  assert.equal(numberInput(' 12 '), '12')
  assert.equal(numberInput('\t-0\n'), '-0')
  assert.equal(numberInput('\u00851.5\u3000'), '1.5')
  assert.equal(numberInput('1 2'), '1 2', '内部空白保持原样交给校验拒绝')
})

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
