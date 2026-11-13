//! JSON 编解码与复制直接使用原生实现，不依赖 HTTP 或浏览器生命周期。

/** 解析契约规定生产者输出的 JSON 文本；静态类型由调用方与 `T` 的字段形状共同保证。 */
export function decode<T>(text: string): T {
  return JSON.parse(text) as T
}

export function encode(value: unknown): string {
  return JSON.stringify(value)
}

/** 深拷贝：同构往返不改变值类型，因此复制结果与 `T` 一致。 */
export function copy<T>(value: T): T {
  return decode<T>(encode(value))
}
