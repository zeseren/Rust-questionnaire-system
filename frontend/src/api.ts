//! 所有写请求携带 CSRF 标记；网络失败不被误报为操作成功。
import { isSafeNumber, LosslessNumber, parse, stringify } from 'lossless-json'

export class ApiError extends Error {
  constructor(
    public status: number,
    public code: string,
    message: string,
    public question_id: string | null = null,
  ) {
    super(message)
  }
}
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
export function message(error: unknown): string {
  return error instanceof ApiError ? error.message : '操作未完成，请稍后重试。'
}
export function query(values: Record<string, string | number | null | undefined>): string {
  const search = new URLSearchParams()
  Object.entries(values).forEach(([key, value]) => {
    if (value !== '' && value != null) search.set(key, String(value))
  })
  return search.size ? `?${search}` : ''
}
export async function request(path: string, method = 'GET', body?: unknown): Promise<Response> {
  const controller = new AbortController()
  const timer = window.setTimeout(() => controller.abort(), 30_000)
  try {
    const headers: Record<string, string> = { Accept: 'application/json' }
    if (method !== 'GET') headers['X-CSRF-Protection'] = '1'
    if (body !== undefined) headers['Content-Type'] = 'application/json'
    const response = await fetch(path, {
      method,
      headers,
      body: body === undefined ? undefined : encode(body),
      credentials: 'same-origin',
      cache: 'no-store',
      signal: controller.signal,
    })
    if (!response.ok) {
      let detail: { code: string; message: string; question_id?: string | null } | undefined
      try {
        detail = decode<{ error: typeof detail }>(await response.text()).error
      } catch {}
      if (detail?.code === 'unauthorized' && path !== '/api/auth/me')
        window.dispatchEvent(new Event('session-expired'))
      throw new ApiError(
        response.status,
        detail?.code ?? 'request_failed',
        detail?.message ?? '服务暂时无法完成请求，请稍后重试。',
        detail?.question_id ?? null,
      )
    }
    return response
  } catch (error) {
    if (error instanceof ApiError) throw error
    throw new ApiError(
      0,
      'network_error',
      method === 'GET'
        ? '无法连接服务，请检查网络或稍后重试。'
        : '尚未确认操作结果，请保留当前内容后重试。',
    )
  } finally {
    window.clearTimeout(timer)
  }
}
export async function api<T>(path: string, method = 'GET', body?: unknown): Promise<T> {
  const response = await request(path, method, body)
  if (response.status === 204) return undefined as T
  try {
    return decode<T>(await response.text())
  } catch {
    throw new ApiError(
      0,
      'invalid_response',
      '服务响应不完整，尚未确认操作结果，请保留内容后重试。',
    )
  }
}
export async function download(path: string, filename: string) {
  const response = await request(path)
  const blob = await response.blob()
  const url = URL.createObjectURL(blob)
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = filename
  anchor.click()
  window.setTimeout(() => URL.revokeObjectURL(url), 1000)
}
