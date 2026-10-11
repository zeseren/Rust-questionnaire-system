//! 输入与本地校验共用 Unicode 标量、空白规则；身份校验随引擎迁到 wasm。

export function isBlank(value: string): boolean {
  // 与 Rust str::trim 一致：U+0085 是空白，U+FEFF 不是。
  return /^\p{White_Space}*$/u.test(value)
}

export function trimWhitespace(value: string): string {
  return value.replace(/^\p{White_Space}+|\p{White_Space}+$/gu, '')
}

export function validText(value: string): boolean {
  // JS 允许孤立代理项，Rust String 不允许；两端均拒绝 NUL。
  return !/[\0\uD800-\uDFFF]/u.test(value)
}
