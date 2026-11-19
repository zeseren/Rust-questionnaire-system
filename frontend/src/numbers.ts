//! 数字输入修剪空白并校验有限数值格式。
import { trimWhitespace } from './value-contract.ts'

export function numberInput(value: string): string {
  return trimWhitespace(value)
}

/** 权威数字文法；输入判定与错误文案共用这一份定义。 */
const NUMBER_LITERAL = /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/

/** 输入是否已是完整且符合文法的数字；有限性由调用方另行判断。 */
export function isNumberLiteral(raw: string): boolean {
  return NUMBER_LITERAL.test(raw)
}

export function numberError(raw: string): string | undefined {
  // 调用方已由 numberInput 完成空白修剪，下方文法也拒绝空白。
  if (!isNumberLiteral(raw) || !Number.isFinite(Number(raw)))
    return '请输入有限数字，例如 12、0.5 或 1e3。'
  return undefined
}
