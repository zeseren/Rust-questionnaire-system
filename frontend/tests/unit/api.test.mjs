//! `api.ts` 的传输契约：请求头与凭据策略、错误信封还原、网络/超时失败文案与下载链路。
//! 所有断言只覆盖模块自身的行为，不依赖真实网络或浏览器。
import assert from 'node:assert/strict'
import test from 'node:test'
import { ApiError, api, download, message, query } from '../../src/api.ts'
import { installBrowserEnv } from '../helpers/browser.mjs'

const QUESTION_ID = '00000000-0000-4000-8000-000000000001'
const jsonResponse = (body, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })
const errorResponse = (status, error) => jsonResponse({ error }, status)
const isNetworkError = (message) => (error) =>
  error instanceof ApiError &&
  error.status === 0 &&
  error.code === 'network_error' &&
  error.message === message

// message 优先返回 Error 实例的文案，其余输入回退到默认或自定义文案
test('message prefers Error text and falls back for other inputs', () => {
  assert.equal(message(new Error('接口读取失败')), '接口读取失败')
  assert.equal(message('非 Error 值'), '操作未完成，请稍后重试。')
  assert.equal(message(null, '自定义回退'), '自定义回退')
  // 未指定关联题目时必须是明确的 null，调用方据此区分“题目级错误”与“整体错误”。
  assert.equal(new ApiError(500, 'internal_error', '内部错误').question_id, null)
})

// query 过滤空值、保留假值并编码中文与保留字符
test('query drops empty values, keeps falsy values, and encodes Chinese and reserved characters', () => {
  assert.equal(query({}), '')
  assert.equal(query({ a: undefined, b: null, c: '' }), '')
  // 0 与 false 是有效筛选条件，不能按“空值”丢弃。
  assert.equal(
    query({ zero: 0, flag: false, text: '文本' }),
    '?zero=0&flag=false&text=%E6%96%87%E6%9C%AC',
  )
  assert.equal(query({ q: 'a&b=c/d?e f' }), '?q=a%26b%3Dc%2Fd%3Fe+f')
})

// GET 请求不携带 CSRF 标记并固定缓存与凭据策略
test('GET requests omit CSRF and fix cache and credentials policy', async (t) => {
  const env = installBrowserEnv(t)
  let captured
  env.fetch = async (path, init) => {
    captured = { path, init }
    return jsonResponse({ id: QUESTION_ID })
  }

  assert.deepEqual(await api('/api/surveys'), { id: QUESTION_ID })
  assert.equal(captured.path, '/api/surveys')
  assert.equal(captured.init.method, 'GET')
  assert.deepEqual(captured.init.headers, { Accept: 'application/json' })
  assert.equal(captured.init.body, undefined)
  assert.equal(captured.init.credentials, 'same-origin')
  assert.equal(captured.init.cache, 'no-store')
  assert.ok(captured.init.signal instanceof AbortSignal)
})

// 写请求统一附加 CSRF 标记，仅在携带请求体时声明 JSON
test('write requests attach CSRF and declare JSON only with a body', async (t) => {
  const env = installBrowserEnv(t)
  const calls = []
  env.fetch = async (_path, init) => {
    calls.push(init)
    return jsonResponse({ ok: true })
  }

  for (const method of ['POST', 'PUT', 'PATCH', 'DELETE']) await api('/api/surveys/1', method)
  for (const init of calls) {
    assert.equal(init.headers['X-CSRF-Protection'], '1')
    assert.equal(init.headers['Content-Type'], undefined)
    assert.equal(init.body, undefined)
  }

  await api('/api/surveys/1/draft', 'PUT', { revision: 2 })
  const withBody = calls.at(-1)
  assert.equal(withBody.method, 'PUT')
  assert.equal(withBody.headers['X-CSRF-Protection'], '1')
  assert.equal(withBody.headers['Content-Type'], 'application/json')
  assert.equal(withBody.body, JSON.stringify({ revision: 2 }))
})

// 写请求可窄传预期账号，同时保留 CSRF 与 JSON 头
test('write requests can declare an expected account while keeping CSRF and JSON headers', async (t) => {
  const env = installBrowserEnv(t)
  let captured
  env.fetch = async (path, init) => {
    captured = { path, init }
    return jsonResponse({ status: 'submitted' })
  }
  const payload = { version_id: QUESTION_ID, idempotency_key: QUESTION_ID, answers: [] }

  await api('/api/public/surveys/1/responses', 'POST', payload, { expectedAccount: QUESTION_ID })

  assert.equal(captured.path, '/api/public/surveys/1/responses')
  assert.equal(captured.init.method, 'POST')
  assert.equal(captured.init.headers['X-Expected-Account'], QUESTION_ID)
  // 窄选项只能追加声明头，认证与内容类型策略仍由传输层统一决定。
  assert.equal(captured.init.headers['X-CSRF-Protection'], '1')
  assert.equal(captured.init.headers['Content-Type'], 'application/json')
  assert.equal(captured.init.body, JSON.stringify(payload))
})

// 未声明预期账号时不发送该头，匿名提交保持原请求形状
test('expected account header is omitted when unset and anonymous submissions keep their request shape', async (t) => {
  const env = installBrowserEnv(t)
  const calls = []
  env.fetch = async (_path, init) => {
    calls.push(init)
    return jsonResponse({ status: 'submitted' })
  }

  await api('/api/public/surveys/1/responses', 'POST', {})
  assert.equal(calls.at(-1).headers['X-Expected-Account'], undefined)
  assert.equal(calls.at(-1).headers['X-CSRF-Protection'], '1')

  await api('/api/public/surveys/1/responses', 'POST', {}, { expectedAccount: '' })
  assert.equal(calls.at(-1).headers['X-Expected-Account'], undefined, '空账号不得发送头')
  assert.equal(calls.at(-1).headers['X-CSRF-Protection'], '1')
})

// 204 No Content 按无正文处理并返回 undefined
test('204 No Content is treated as bodyless and returns undefined', async (t) => {
  const env = installBrowserEnv(t)
  env.fetch = async () => new Response(null, { status: 204 })
  assert.equal(await api('/api/surveys/1/close', 'POST'), undefined)
})

// 错误信封还原为带题目定位的 ApiError
test('error envelopes become ApiError with question location', async (t) => {
  const env = installBrowserEnv(t)
  env.fetch = async () =>
    errorResponse(422, {
      code: 'validation_failed',
      message: '题目「入口选择」不满足约束',
      question_id: QUESTION_ID,
    })

  await assert.rejects(api('/api/surveys/1/draft', 'PUT', { revision: 1 }), (error) => {
    assert.ok(error instanceof ApiError)
    assert.equal(error.status, 422)
    assert.equal(error.code, 'validation_failed')
    assert.equal(error.message, '题目「入口选择」不满足约束')
    assert.equal(error.question_id, QUESTION_ID)
    return true
  })
})

// 错误体不是 JSON 或缺字段时回退到通用状态文案
test('non-JSON or incomplete error bodies fall back to generic status text', async (t) => {
  const env = installBrowserEnv(t)
  for (const [body, status] of [
    ['<html>网关错误</html>', 502],
    [JSON.stringify({ error: {} }), 429],
  ]) {
    env.fetch = async () => new Response(body, { status })
    await assert.rejects(
      api('/api/surveys'),
      (error) =>
        error instanceof ApiError &&
        error.status === status &&
        error.code === 'request_failed' &&
        error.message === '服务暂时无法完成请求，请稍后重试。' &&
        error.question_id === null,
      String(status),
    )
  }
})

// 非认证探测请求的 unauthorized 触发一次会话过期事件
test('unauthorized on non-auth-probe requests emits one session expiry event', async (t) => {
  const env = installBrowserEnv(t)
  let expired = 0
  env.window.addEventListener('session-expired', () => {
    expired += 1
  })
  env.fetch = async () => errorResponse(401, { code: 'unauthorized', message: '登录已过期' })

  await assert.rejects(
    api('/api/surveys'),
    (error) => error instanceof ApiError && error.status === 401,
  )
  assert.equal(expired, 1)
})

// 认证探测请求的 401 不触发会话过期事件
test('401 on auth probe requests does not emit a session expiry event', async (t) => {
  const env = installBrowserEnv(t)
  let expired = 0
  env.window.addEventListener('session-expired', () => {
    expired += 1
  })
  env.fetch = async () => errorResponse(401, { code: 'unauthorized', message: '未登录' })

  await assert.rejects(
    api('/api/auth/me'),
    (error) => error instanceof ApiError && error.status === 401 && error.code === 'unauthorized',
  )
  assert.equal(expired, 0)
})

// 网络异常按读写语义给出不同文案
test('network failures use distinct messages for read and write semantics', async (t) => {
  const env = installBrowserEnv(t)
  env.fetch = async () => {
    throw new TypeError('fetch failed')
  }

  await assert.rejects(api('/api/surveys'), isNetworkError('无法连接服务，请检查网络或稍后重试。'))
  await assert.rejects(
    api('/api/surveys', 'POST', {}),
    isNetworkError('尚未确认操作结果，请保留当前内容后重试。'),
  )
})

// 30 秒截止会中止请求、归为网络失败并清除定时器
test('30-second deadline aborts the request as a network failure and clears the timer', async (t) => {
  const env = installBrowserEnv(t, { timers: 'manual' })
  env.fetch = (_path, init) =>
    new Promise((_resolve, reject) => {
      init.signal.addEventListener('abort', () => reject(new DOMException('已中止', 'AbortError')))
    })

  const pending = api('/api/slow')
  const [timer] = env.timers.pending
  assert.equal(timer.delay, 30_000)
  const assertion = assert.rejects(pending, isNetworkError('无法连接服务，请检查网络或稍后重试。'))
  timer.callback()
  await assertion
  assert.deepEqual(env.timers.pending, [])
})

// 成功响应正文非法 JSON 时给出无效响应错误
test('invalid JSON in a successful response body yields an invalid response error', async (t) => {
  const env = installBrowserEnv(t)
  env.fetch = async () => new Response('{ 不是 JSON', { status: 200 })

  await assert.rejects(
    api('/api/surveys'),
    (error) =>
      error instanceof ApiError &&
      error.status === 0 &&
      error.code === 'invalid_response' &&
      error.message === '服务响应不完整，尚未确认操作结果，请保留内容后重试。',
  )
})

// download 以 GET 拉取并触发锚点下载，延时撤销对象地址
test('download fetches via GET, triggers an anchor download, and revokes the object URL later', async (t) => {
  const env = installBrowserEnv(t, { timers: 'manual' })
  let request
  env.fetch = async (path, init) => {
    request = { path, init }
    return new Response('回执编号\n', { status: 200 })
  }

  await download('/api/surveys/1/export.csv', '答卷.csv')

  assert.equal(request.path, '/api/surveys/1/export.csv')
  assert.equal(request.init.method, 'GET')
  assert.equal(request.init.headers['X-CSRF-Protection'], undefined)
  const [anchor] = env.anchors
  assert.equal(anchor.href, env.objectUrls.created[0])
  assert.equal(anchor.download, '答卷.csv')
  assert.equal(anchor.clicks, 1)

  env.timers.run()
  assert.deepEqual(env.objectUrls.revoked, [anchor.href])
})

// download 遇到错误信封时抛出 ApiError 且不创建下载链路
test('download throws ApiError on error envelopes and creates no download chain', async (t) => {
  const env = installBrowserEnv(t, { timers: 'manual' })
  env.fetch = async () =>
    errorResponse(409, {
      code: 'version_changed',
      message: '版本已更新，请刷新后重新导出',
      question_id: null,
    })

  await assert.rejects(download('/api/surveys/1/export.csv', '答卷.csv'), (error) => {
    assert.ok(error instanceof ApiError)
    assert.equal(error.status, 409)
    assert.equal(error.code, 'version_changed')
    assert.equal(error.message, '版本已更新，请刷新后重新导出')
    return true
  })

  // 错误响应不得触发浏览器下载：不创建锚点、不生成对象地址，也不遗留截止定时器。
  assert.deepEqual(env.anchors, [])
  assert.deepEqual(env.objectUrls.created, [])
  assert.deepEqual(env.timers.pending, [])
})

// download 读取响应正文失败时报告中文网络错误并清理截止定时器
test('download reports a Chinese network error when reading the body fails and clears the deadline timer', async (t) => {
  const env = installBrowserEnv(t, { timers: 'manual' })
  env.fetch = async () => ({
    ok: true,
    status: 200,
    blob: async () => {
      throw new Error('连接中断')
    },
  })

  await assert.rejects(
    download('/api/surveys/1/export.csv', '答卷.csv'),
    isNetworkError('下载未完成，请检查网络后重试。'),
  )

  // 下载未完成时不得创建锚点或对象地址，且定时器已经清除。
  assert.deepEqual(env.anchors, [])
  assert.deepEqual(env.objectUrls.created, [])
  assert.deepEqual(env.timers.pending, [])
})
