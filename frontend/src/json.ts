//! JSON 编解码与复制保留数字原文，不依赖 HTTP 或浏览器生命周期。
import { isSafeNumber, LosslessNumber, parse, stringify } from 'lossless-json'

/**
 * 反序列化边界：`lossless-json` 不携带静态类型信息，`as T` 的安全前提完全由调用方保证——
 * 文本必须来自契约规定的生产者（服务端响应或本模块先前的 `encode` 输出），字段形状与 `T` 一致。
 */
export function decode<T>(text: string): T {
  return parse(text, undefined, {
    parseNumber: (value) => (isSafeNumber(value) ? Number(value) : new LosslessNumber(value)),
  }) as T
}
export function encode(value: unknown): string {
  return stringify(value) ?? 'null'
}
/**
 * 深拷贝边界：输入先序列化再按同一契约反序列化，因此结果与 `T` 同构；
 * 这是唯一无需调用方额外证明的 `as T` 场景（往返不改变值类型）。
 */
export function copy<T>(value: T): T {
  return decode<T>(encode(value))
}

/**
 * 冻结提交载荷时保留数字的传输写法，避免指数被展开成超范围裸整数。
 * `as T` 的安全前提：`value` 本身已是 `T`，序列化只改变数字的文本表示而不改变结构。
 */
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
    if (input instanceof LosslessNumber || input === null || typeof input !== 'object') return input
    if (Array.isArray(input)) {
      // `Array.isArray` 会把 `unknown` 宽化成元素类型未受检的数组；先落到 `readonly unknown[]` 阻断污染。
      const items: readonly unknown[] = input
      return items.map((item) => rewrite(item))
    }
    if (typeof input === 'object' && input !== null) {
      // 已排除数组；`object` 收窄不足以提供字段类型，按 JSON 对象契约断言为字符串键记录。
      const record = input as Record<string, unknown>
      return Object.fromEntries(Object.entries(record).map(([key, item]) => [key, rewrite(item)]))
    }
    return input
  }
  return stringify(rewrite(value)) ?? 'null'
}
