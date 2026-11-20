//! 未决提交与草稿的冻结作用域：匿名固定 anonymous，登录限次绑定题面载入时的账号。
//! 作用域只存在于存储键中；载荷本身只保存 Submission，不重复记录账号。
import { isSubmission } from './answers.ts'
import type { FillingMode, Submission } from './types.ts'

const STORAGE_PREFIX = 'wenxu:fill-draft:'
export const ANONYMOUS_SCOPE = 'anonymous'

/** 内存中的冻结提交：`scope` 来自读取时的存储键，而不是载荷内容。 */
export interface FrozenPending {
  scope: string
  payload: Submission
}

/** 匿名固定 anonymous；登录限次缺账号时返回 null，绝不降级到匿名作用域。 */
export function scopeFor(mode: FillingMode | undefined, accountId: string | null): string | null {
  return mode === 'login_once' ? accountId : ANONYMOUS_SCOPE
}

/** 匿名草稿沿用按版本隔离的键；登录限次再追加账号，避免 B 读到 A 的原文。 */
export function draftKeyFor(surveyId: string, versionId: string, scope: string): string {
  return scope === ANONYMOUS_SCOPE
    ? `${STORAGE_PREFIX}${surveyId}:${versionId}`
    : `${STORAGE_PREFIX}${surveyId}:${versionId}:${scope}`
}

export function pendingKeyFor(surveyId: string, scope: string): string {
  return `${STORAGE_PREFIX}${surveyId}:pending:${scope}`
}

/**
 * 从不可信本地存储恢复未决提交：只接受完整 Submission，
 * 并用调用方指定的存储键作用域包装成内存冻结对象。
 */
export function freezePending(value: unknown, scope: string): FrozenPending | null {
  return isSubmission(value) ? { scope, payload: value } : null
}
