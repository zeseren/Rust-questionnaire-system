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

test('ApiError 保留状态码、错误码、消息与关联题目', () => {
  const error = new ApiError(422, 'validation_failed', '题干不能为空', QUESTION_ID)
  assert.ok(error instanceof Error)
  assert.equal(error.status, 422)
  assert.equal(error.code, 'validation_failed')
  assert.equal(error.message, '题干不能为空')
  assert.equal(error.question_id, QUESTION_ID)
  // 未指定关联题目时必须是明确的 null，调用方据此区分“题目级错误”与“整体错误”。
  assert.equal(new ApiError(500, 'internal_error', '内部错误').question_id, null)
})

test('message 优先返回 Error 实例的文案，其余输入回退到默认或自定义文案', () => {
  assert.equal(message(new Error('接口读取失败')), '接口读取失败')
  assert.equal(message('非 Error 值'), '操作未完成，请稍后重试。')
  assert.equal(message(null, '自定义回退'), '自定义回退')
})

test('query 过滤空值、保留假值并编码中文与保留字符', () => {
  assert.equal(query({}), '')
  assert.equal(query({ a: undefined, b: null, c: '' }), '')
  // 0 与 false 是有效筛选条件，不能按“空值”丢弃。
  assert.equal(
    query({ zero: 0, flag: false, text: '文本' }),
    '?zero=0&flag=false&text=%E6%96%87%E6%9C%AC',
  )
  assert.equal(query({ q: 'a&b=c/d?e f' }), '?q=a%26b%3Dc%2Fd%3Fe+f')
})

test('GET 请求不携带 CSRF 标记并固定缓存与凭据策略', async (t) => {
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

test('写请求统一附加 CSRF 标记，仅在携带请求体时声明 JSON', async (t) => {
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

test('204 No Content 按无正文处理并返回 undefined', async (t) => {
  const env = installBrowserEnv(t)
  env.fetch = async () => new Response(null, { status: 204 })
  assert.equal(await api('/api/surveys/1/close', 'POST'), undefined)
})

test('错误信封还原为带题目定位的 ApiError', async (t) => {
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

test('错误体不是 JSON 时回退到通用状态文案', async (t) => {
  const env = installBrowserEnv(t)
  env.fetch = async () => new Response('<html>网关错误</html>', { status: 502 })

  await assert.rejects(
    api('/api/surveys'),
    (error) =>
      error instanceof ApiError &&
      error.status === 502 &&
      error.code === 'request_failed' &&
      error.message === '服务暂时无法完成请求，请稍后重试。' &&
      error.question_id === null,
  )
})

test('错误信封缺失字段时逐字段回退，不伪造错误码或题目', async (t) => {
  const env = installBrowserEnv(t)
  env.fetch = async () => jsonResponse({ error: {} }, 429)

  await assert.rejects(
    api('/api/auth/login', 'POST', { username: 'alice', password: 'secret' }),
    (error) =>
      error instanceof ApiError &&
      error.status === 429 &&
      error.code === 'request_failed' &&
      error.message === '服务暂时无法完成请求，请稍后重试。' &&
      error.question_id === null,
  )
})

test('非认证探测请求的 unauthorized 触发一次会话过期事件', async (t) => {
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

test('认证探测请求的 401 不触发会话过期事件', async (t) => {
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

test('网络异常按读写语义给出不同文案', async (t) => {
  const env = installBrowserEnv(t)
  env.fetch = async () => {
    throw new TypeError('fetch failed')
  }

  await assert.rejects(
    api('/api/surveys'),
    (error) =>
      error instanceof ApiError &&
      error.status === 0 &&
      error.code === 'network_error' &&
      error.message === '无法连接服务，请检查网络或稍后重试。',
  )
  await assert.rejects(
    api('/api/surveys', 'POST', {}),
    (error) =>
      error instanceof ApiError &&
      error.status === 0 &&
      error.code === 'network_error' &&
      error.message === '尚未确认操作结果，请保留当前内容后重试。',
  )
})

test('30 秒截止会中止请求、归为网络失败并清除定时器', async (t) => {
  const env = installBrowserEnv(t, { timers: 'manual' })
  env.fetch = (_path, init) =>
    new Promise((_resolve, reject) => {
      init.signal.addEventListener('abort', () => reject(new DOMException('已中止', 'AbortError')))
    })

  const pending = api('/api/slow')
  const [timer] = env.timers.pending
  assert.equal(timer.delay, 30_000)
  const assertion = assert.rejects(
    pending,
    (error) =>
      error instanceof ApiError &&
      error.status === 0 &&
      error.code === 'network_error' &&
      error.message === '无法连接服务，请检查网络或稍后重试。',
  )
  timer.callback()
  await assertion
  assert.deepEqual(env.timers.cleared, [timer.id])
})

test('成功响应正文非法 JSON 时给出无效响应错误', async (t) => {
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

test('download 以 GET 获取正文、触发锚点点击并延时释放对象地址', async (t) => {
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
  assert.equal(env.objectUrls.created.length, 1)
  assert.equal(await env.objectUrls.blobs[0].text(), '回执编号\n')
  const [anchor] = env.anchors
  assert.equal(anchor.href, env.objectUrls.created[0])
  assert.equal(anchor.download, '答卷.csv')
  assert.equal(anchor.clicks, 1)
  assert.deepEqual(env.objectUrls.revoked, [])

  const [timer] = env.timers.pending
  assert.equal(timer.delay, 1000)
  env.timers.run()
  assert.deepEqual(env.objectUrls.revoked, [env.objectUrls.created[0]])
})

test('download 遇到错误信封时抛出 ApiError 且不创建下载链路', async (t) => {
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

  // 错误响应不得触发浏览器下载：不创建锚点、不生成对象地址，并立即清除截止定时器。
  assert.deepEqual(env.anchors, [])
  assert.deepEqual(env.objectUrls.created, [])
  assert.deepEqual(env.timers.pending, [])
  assert.equal(env.timers.cleared.length, 1)
})
