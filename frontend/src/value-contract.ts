//! 输入与 DSL 共用 Unicode 标量、空白和配置身份规则。

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

export function isConfigurationId(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    value.length === 36 &&
    /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(value) &&
    value !== '00000000-0000-0000-0000-000000000000'
  )
}
