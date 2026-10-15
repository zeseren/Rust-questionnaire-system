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

/** 冻结提交载荷时保留数字的传输写法，避免指数被展开成超范围裸整数。 */
export function copyPayload<T>(value: T): T {
  return parse(encode(value), undefined, { parseNumber: (value) => new LosslessNumber(value) }) as T
}

/**
 * 规则求值/事件载荷的答案编码：原生 number 已是浮点数，用指数形式保留其数值，
 * 不把十进制显示值重新当作原始裸整数校验。LosslessNumber 仍保留输入原文，
 * 由核心侧拒绝超范围或无法精确表示的原始整数。
 */
export function encodeAnswers(value: unknown): string {
  const normalized = (raw: number): LosslessNumber => {
    if (!Number.isFinite(raw)) throw new Error('答案数字必须是有限值')
    return new LosslessNumber(raw.toExponential())
  }
  const rewrite = (input: unknown): unknown => {
    if (typeof input === 'number') return normalized(input)
    if (Array.isArray(input)) return input.map(rewrite)
    if (input instanceof LosslessNumber || input === null || typeof input !== 'object') return input
    return Object.fromEntries(Object.entries(input).map(([key, item]) => [key, rewrite(item)]))
  }
  return stringify(rewrite(value)) ?? 'null'
}
