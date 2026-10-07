//! 数字先检查传输原文的长度和整数精度，再交给题型或表达式使用。
import { trimWhitespace } from './value-contract.ts'

// 与 Rust domain::finite_number 的 JSON 表示预算和裸整数范围一致。
const MAX_NUMBER_JSON_BYTES = 64
const MIN_JSON_INTEGER = -(1n << 63n)
const MAX_JSON_INTEGER = (1n << 64n) - 1n

export function numberInput(value: string): string {
  return trimWhitespace(value).replace(/([eE])(?=\d)/, '$1+')
}

export function numberError(raw: string): string | undefined {
  // serde_json 会补齐正指数符号；DSL 常量本身不允许首尾空白。
  if (raw.replace(/([eE])(?=\d)/, '$1+').length > MAX_NUMBER_JSON_BYTES)
    return `数字最多 ${MAX_NUMBER_JSON_BYTES} 个字符（含小数点和指数符号）。`
  if (
    raw !== trimWhitespace(raw) ||
    !/^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/.test(raw) ||
    !Number.isFinite(Number(raw))
  )
    return '请输入有限数字，例如 12、0.5 或 1e3。'
  if (!/[.eE]/.test(raw)) {
    const integer = BigInt(raw)
    if (integer < MIN_JSON_INTEGER || integer > MAX_JSON_INTEGER) return '整数超出支持的数值范围。'
    if (integer !== BigInt(Number(raw))) return '整数无法精确表示，请调整数值。'
  }
  return undefined
}

export function numeric(raw: string): number | undefined {
  if (numberError(raw)) return undefined
  const value = Number(raw)
  return value === 0 ? 0 : value
}
