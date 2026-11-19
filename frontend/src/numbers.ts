//! 数字输入修剪空白并校验有限数值格式。
import { trimWhitespace } from './value-contract.ts'

export function numberInput(value: string): string {
  return trimWhitespace(value)
}

/** 权威数字文法；输入判定与错误文案共用这一份定义。 */
const NUMBER_LITERAL = /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/
/** 同一文法的前缀：还能补全成合法数字的输入，如 `-`、`12.`、`1e-`；`.5`、`01` 永远补不成。 */
const NUMBER_PREFIX = /^-?(?:(?:0|[1-9]\d*)(?:\.\d*|(?:\.\d+)?[eE][+-]?\d*)?)?$/

/** 输入尚未完成但仍可能补全成合法数字；空串不算输入中。 */
export function isNumberPrefix(raw: string): boolean {
  return raw !== '' && !NUMBER_LITERAL.test(raw) && NUMBER_PREFIX.test(raw)
}

export function numberError(raw: string): string | undefined {
  // 调用方已由 numberInput 完成空白修剪，下方文法也拒绝空白。
  if (!NUMBER_LITERAL.test(raw) || !Number.isFinite(Number(raw)))
    return '请输入有限数字，例如 12、0.5 或 1e3。'
  return undefined
}
