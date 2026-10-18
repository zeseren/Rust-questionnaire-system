//! 数字输入修剪空白并校验有限数值格式。
import { trimWhitespace } from './value-contract.ts'

export function numberInput(value: string): string {
  return trimWhitespace(value)
}

export function numberError(raw: string): string | undefined {
  // 调用方已由 numberInput 完成空白修剪，下方正则也拒绝空白。
  if (!/^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/.test(raw) || !Number.isFinite(Number(raw)))
    return '请输入有限数字，例如 12、0.5 或 1e3。'
  return undefined
}
