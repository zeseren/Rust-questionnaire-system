//! 界面与引擎共用 Unicode 空白规则；答案合法性由 wasm 引擎判定。

export function isBlank(value: string): boolean {
  // 与 Rust str::trim 一致：U+0085 是空白，U+FEFF 不是。
  return /^\p{White_Space}*$/u.test(value)
}

export function trimWhitespace(value: string): string {
  return value.replace(/^\p{White_Space}+|\p{White_Space}+$/gu, '')
}
