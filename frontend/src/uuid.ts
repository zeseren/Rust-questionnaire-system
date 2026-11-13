//! 前端本地身份统一由 RFC 9562 UUIDv7 生成，后端按非零标准连字符 UUID 校验。
//! 48 位毫秒时间戳在前，同一毫秒内依靠 74 位随机位保持唯一，刷新/重开后仍大致按创建先后排序。

/** 生成 UUIDv7 文本（小写、带连字符），与 `crypto.randomUUID()` 输出格式相同。 */
export function newId(): string {
  // 浏览器与 Node 19+ 都提供全局 `crypto.getRandomValues`，无需环境分支。
  const bytes = crypto.getRandomValues(new Uint8Array(16))
  // 时间戳写入前 6 字节（大端）；数值范围到公元 10889 年，不会溢出。
  let timestamp = Date.now()
  for (let index = 5; index >= 0; index -= 1) {
    bytes[index] = timestamp % 256
    timestamp = Math.floor(timestamp / 256)
  }
  // 第 7 字节高 4 位是版本号 7，第 9 字节高 2 位是 RFC 变体 10。
  bytes[6] = (bytes[6]! & 0x0f) | 0x70
  bytes[8] = (bytes[8]! & 0x3f) | 0x80
  let text = ''
  for (const [index, byte] of bytes.entries()) {
    if (index === 4 || index === 6 || index === 8 || index === 10) text += '-'
    text += byte.toString(16).padStart(2, '0')
  }
  return text
}
