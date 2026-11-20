//! 数字输入的十进制文法校验：只接受可精确表示的有限双精度写法。
/** 权威数字文法；输入判定与错误文案共用这一份定义。 */
const NUMBER_LITERAL = /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/
/** 同一文法的前缀：还能补全成合法数字的输入，如 `-`、`12.`、`1e-`；`.5`、`01` 永远补不成。 */
const NUMBER_PREFIX = /^-?(?:(?:0|[1-9]\d*)(?:\.\d*|(?:\.\d+)?[eE][+-]?\d*)?)?$/

/** 输入尚未完成但仍可能补全成合法数字；空串不算输入中。 */
export function isNumberPrefix(raw: string): boolean {
  return raw !== '' && !NUMBER_LITERAL.test(raw) && NUMBER_PREFIX.test(raw)
}

/** 完整输入能否按同一文法转成可精确表示的有限数字；具体修正文案由引擎按题级给出。 */
export function isNumberLiteral(raw: string): boolean {
  // 调用方已由 trimWhitespace 完成空白修剪，下方文法也拒绝空白。
  return NUMBER_LITERAL.test(raw) && Number.isFinite(Number(raw))
}
