//! JSON 编解码与复制保留数字原文，不依赖 HTTP 或浏览器生命周期。
import { isSafeNumber, LosslessNumber, parse, stringify } from 'lossless-json'

export function decode<T>(text: string): T {
  return parse(text, undefined, {
    parseNumber: (value) => (isSafeNumber(value) ? Number(value) : new LosslessNumber(value)),
  }) as T
}
export function encode(value: unknown): string {
  return stringify(value) ?? 'null'
}
export function copy<T>(value: T): T {
  return decode<T>(encode(value))
}
