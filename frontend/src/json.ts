//! JSON 编解码与复制直接使用原生实现，不依赖 HTTP 或浏览器生命周期。
/**
 * 反序列化边界：`JSON.parse` 不携带静态类型信息，`as T` 的安全前提完全由调用方保证——
 * 文本必须来自契约规定的生产者（服务端响应或本模块先前的 `encode` 输出），字段形状与 `T` 一致。
 */
export function decode<T>(text: string): T {
  return JSON.parse(text) as T
}
export function encode(value: unknown): string {
  return JSON.stringify(value)
}
/**
 * 深拷贝边界：输入先序列化再按同一契约反序列化，因此结果与 `T` 同构；
 * 这是唯一无需调用方额外证明的 `as T` 场景（往返不改变值类型）。
 */
export function copy<T>(value: T): T {
  return decode<T>(encode(value))
}
