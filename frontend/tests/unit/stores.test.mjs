//! Pinia stores 的真实状态迁移：会话恢复/登录/退出、外观偏好与提示计时器。
//! 每个用例使用独立的 `createPinia()`，避免 store 单例跨用例泄漏状态。
import assert from 'node:assert/strict'
import test from 'node:test'
import { createPinia, setActivePinia } from 'pinia'
import { ApiError } from '../../src/api.ts'
import { useAppearance, useAuth, useNotice } from '../../src/stores.ts'
import { installBrowserEnv } from '../helpers/browser.mjs'

const USER = {
  id: '00000000-0000-4000-8000-000000000001',
  username: 'alice',
  role: 'user',
}
const jsonResponse = (body, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })
const errorResponse = (status, error) => jsonResponse({ error }, status)
const freshPinia = () => setActivePinia(createPinia())

function deferred() {
  let resolve, reject
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}

test('restore 成功写入用户，并让并发调用共享同一个挂起请求', async (t) => {
  const env = installBrowserEnv(t)
  let calls = 0
  const pending = deferred()
  env.fetch = () => {
    calls += 1
    return pending.promise
  }
  freshPinia()
  const auth = useAuth()

  const first = auth.restore()
  const second = auth.restore()
  assert.equal(calls, 1)

  pending.resolve(jsonResponse(USER))
  await Promise.all([first, second])
  assert.deepEqual(auth.user, USER)
  assert.equal(auth.restoreError, '')
  assert.equal(auth.expired, false)

  // 初始化完成后不再探测服务端。
  await auth.restore()
  assert.equal(calls, 1)
})

test('/api/auth/me 返回 401 时静默置空用户且不记录恢复错误', async (t) => {
  const env = installBrowserEnv(t)
  env.fetch = async (path) => {
    assert.equal(path, '/api/auth/me')
    return errorResponse(401, { code: 'unauthorized', message: '未登录' })
  }
  freshPinia()
  const auth = useAuth()

  await auth.restore()
  assert.equal(auth.user, null)
  assert.equal(auth.restoreError, '')
  assert.equal(auth.expired, false)
})

test('restore 遇到非 401 失败时记录后端文案', async (t) => {
  const env = installBrowserEnv(t)
  env.fetch = async () => errorResponse(500, { code: 'internal_error', message: '数据库暂不可用' })
  freshPinia()
  const auth = useAuth()

  await auth.restore()
  assert.equal(auth.user, null)
  assert.equal(auth.restoreError, '数据库暂不可用')
})

test('login 成功后更新用户并清除过期与会话恢复错误', async (t) => {
  const env = installBrowserEnv(t)
  let logins = 0
  env.fetch = async (path, init) => {
    if (path === '/api/auth/me')
      return errorResponse(500, { code: 'internal_error', message: '会话恢复失败' })
    logins += 1
    assert.deepEqual(JSON.parse(init.body), { username: 'alice', password: 'secret' })
    return jsonResponse(USER)
  }
  freshPinia()
  const auth = useAuth()

  await auth.restore()
  assert.equal(auth.restoreError, '会话恢复失败')
  auth.expired = true

  await auth.login('alice', 'secret')
  assert.equal(logins, 1)
  assert.deepEqual(auth.user, USER)
  assert.equal(auth.expired, false)
  assert.equal(auth.restoreError, '')
})

test('login 失败抛出领域错误且不清除现有会话', async (t) => {
  const env = installBrowserEnv(t)
  env.fetch = async (path) => {
    if (path === '/api/auth/me') return jsonResponse(USER)
    return errorResponse(401, { code: 'unauthorized', message: '用户名或密码错误' })
  }
  freshPinia()
  const auth = useAuth()

  await auth.restore()
  await assert.rejects(
    auth.login('alice', 'wrong'),
    (error) =>
      error instanceof ApiError &&
      error.status === 401 &&
      error.code === 'unauthorized' &&
      error.message === '用户名或密码错误',
  )
  assert.deepEqual(auth.user, USER)
  assert.equal(auth.expired, false)
})

test('logout 清空用户与过期标记', async (t) => {
  const env = installBrowserEnv(t)
  env.fetch = async (path) => {
    if (path === '/api/auth/me') return jsonResponse(USER)
    assert.equal(path, '/api/auth/logout')
    return new Response(null, { status: 204 })
  }
  freshPinia()
  const auth = useAuth()

  await auth.restore()
  auth.expired = true
  await auth.logout()
  assert.equal(auth.user, null)
  assert.equal(auth.expired, false)
})

test('外观偏好读取本地存储中的合法值', (t) => {
  const env = installBrowserEnv(t)
  env.localStorage.setItem('wenxu-theme', 'dark')
  freshPinia()

  const appearance = useAppearance()
  assert.equal(appearance.preference, 'dark')
  assert.equal(appearance.dark, true)
})

test('外观偏好忽略非法值并回退到跟随系统', (t) => {
  const env = installBrowserEnv(t)
  env.localStorage.setItem('wenxu-theme', 'sepia')
  freshPinia()

  const appearance = useAppearance()
  assert.equal(appearance.preference, 'system')
  assert.equal(appearance.dark, false)
})

test('本地存储读写抛错时外观偏好安全降级', (t) => {
  installBrowserEnv(t, { storageThrows: true })
  freshPinia()

  const appearance = useAppearance()
  assert.equal(appearance.preference, 'system')
  // 写入抛错被吞掉，但内存中的偏好仍必须生效。
  appearance.setTheme('dark')
  assert.equal(appearance.preference, 'dark')
  assert.equal(appearance.dark, true)
})

test('setTheme 同步 data-theme、theme-color 与本地存储', (t) => {
  const env = installBrowserEnv(t)
  freshPinia()
  const appearance = useAppearance()

  appearance.setTheme('dark')
  assert.equal(env.document.documentElement.dataset.theme, 'dark')
  assert.equal(env.meta.getAttribute('content'), '#121210')
  assert.equal(env.localStorage.getItem('wenxu-theme'), 'dark')

  appearance.setTheme('light')
  assert.equal(env.document.documentElement.dataset.theme, 'light')
  assert.equal(env.meta.getAttribute('content'), '#f5f2eb')
  assert.equal(env.localStorage.getItem('wenxu-theme'), 'light')

  appearance.setTheme('system')
  assert.equal(env.document.documentElement.dataset.theme, 'light')
  assert.equal(env.localStorage.getItem('wenxu-theme'), 'system')
})

test('系统深色偏好切换联动，显式浅色不受系统影响', (t) => {
  const env = installBrowserEnv(t)
  freshPinia()
  const appearance = useAppearance()
  assert.equal(appearance.dark, false)

  env.setSystemDark(true)
  assert.equal(appearance.dark, true)

  env.setSystemDark(false)
  assert.equal(appearance.dark, false)

  appearance.setTheme('light')
  env.setSystemDark(true)
  assert.equal(appearance.dark, false)
})

test('连续 show 重置计时器，只按最后一次提示计时', (t) => {
  t.mock.timers.enable({ apis: ['setTimeout'] })
  freshPinia()
  const notice = useNotice()

  notice.show('第一次提示')
  t.mock.timers.tick(3000)
  notice.show('第二次提示')
  t.mock.timers.tick(4499)
  assert.equal(notice.text, '第二次提示')
  t.mock.timers.tick(1)
  assert.equal(notice.text, '')
})
