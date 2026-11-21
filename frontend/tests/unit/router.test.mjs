//! 路由基础设施契约：`safeReturn` 的开放重定向防护与路由表元数据完整性。
//! `router.ts` 在模块求值时就调用 `createWebHistory()`，因此必须先装好浏览器桩再动态导入。
import assert from 'node:assert/strict'
import test, { after, before } from 'node:test'
import { createPinia, setActivePinia } from 'pinia'
import { useNotice } from '../../src/stores.ts'
import { installBrowserEnv } from '../helpers/browser.mjs'

const USER = {
  id: '00000000-0000-4000-8000-000000000001',
  username: 'alice',
  role: 'user',
}
const ADMIN = { ...USER, id: '00000000-0000-4000-8000-000000000002', role: 'admin' }
const jsonResponse = (body, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })
const errorResponse = (status, error) => jsonResponse({ error }, status)

let env
let safeReturn
let router

before(async () => {
  env = installBrowserEnv(undefined, { pathname: '/surveys' })
  ;({ safeReturn, router } = await import('../../src/router.ts'))
  // 导航会解析懒加载的 .vue 视图，Node 无法加载 SFC；用空组件替换被测路由，
  // 保留名称、路径与 meta，从而只验证守卫与 afterEach。
  const Stub = { render: () => null }
  for (const route of [
    { path: '/', name: 'home', meta: { auth: true, title: '工作台' } },
    { path: '/surveys', name: 'surveys', meta: { auth: true, title: '我的问卷' } },
    { path: '/login', name: 'login', meta: { standalone: true, title: '登录' } },
    { path: '/admin', name: 'admin', meta: { auth: true, admin: true, title: '平台管理' } },
  ])
    router.addRoute({ ...route, component: Stub })
})

after(() => {
  env?.restore()
})

// safeReturn 只放行以单个斜杠开头的站内路径
test('safeReturn allows only site paths starting with a single slash', () => {
  const accepted = [
    '/',
    '/surveys',
    '/surveys/123/edit?tab=flow',
    '/login-help',
    '/surveys#答卷列表',
    // 编码后的反斜杠仍是普通路径片段，浏览器不会把它还原成新的 authority。
    '/%5cevil',
  ]
  for (const value of accepted) assert.equal(safeReturn(value), value, value)
})

// safeReturn 拒绝非字符串、相对、协议相对、反斜杠与换行路径
test('safeReturn rejects non-string, relative, protocol-relative, backslash and newline paths', () => {
  const rejected = [
    undefined,
    'surveys',
    '///evil.com',
    // 反斜杠会被浏览器规范化成协议相对地址。
    String.raw`\evil`,
    String.raw`/\evil`,
    // `$` 会放过末尾换行，路径必须显式拒绝。
    '/surveys\n',
  ]
  for (const value of rejected) assert.equal(safeReturn(value), '/', String(value))
})

// safeReturn 拒绝回跳登录与注册页自身
test('safeReturn rejects login and register pages as redirect targets', () => {
  const rejected = [
    '/login',
    '/login?redirect=/x',
    '/login#section',
    '/register',
    '/register?a=1',
    '/register#top',
  ]
  for (const value of rejected) assert.equal(safeReturn(value), '/', value)
})

// 路由表配置完整：懒加载、唯一名称与关键权限元数据
test('route table is fully configured: lazy loading, unique names and key permission metadata', () => {
  const routes = router.options.routes
  const byName = new Map(routes.map((route) => [route.name, route]))

  // 每条路由都必须懒加载视图组件，且名称唯一、通配路由排在最后。
  const names = new Set()
  for (const route of routes) {
    assert.equal(typeof route.component, 'function', `${route.name} 必须懒加载视图`)
    assert.equal(names.has(route.name), false, `${route.name} 名称重复`)
    names.add(route.name)
  }
  assert.equal(routes.at(-1).name, 'not-found')

  // 登录、注册、填写与兜底页对外公开，其余业务路由一律要求登录。
  const publicNames = new Set(['login', 'register', 'fill', 'not-found'])
  for (const route of routes) {
    if (!publicNames.has(route.name))
      assert.equal(route.meta.auth, true, `${route.name} 必须要求登录`)
  }
  assert.equal(byName.get('admin').meta.admin, true)
  // 独立布局路由不套用主框架：登录、注册、填写与兜底页。
  for (const name of ['login', 'register', 'fill', 'not-found'])
    assert.equal(byName.get(name).meta.standalone, true, `${name} 必须使用独立布局`)
})

// 未登录访问受保护路由会重定向到登录页并保留回跳地址
test('unauthenticated access to a protected route redirects to login and keeps the return path', async () => {
  setActivePinia(createPinia())
  env.fetch = async () => errorResponse(401, { code: 'unauthorized', message: '未登录' })

  await router.push('/surveys')

  const route = router.currentRoute.value
  assert.equal(route.name, 'login')
  assert.equal(route.path, '/login')
  assert.equal(route.query.redirect, '/surveys')
})

// 普通用户访问平台管理被重定向到工作台并展示权限提示
test('regular user accessing platform admin is redirected to workspace with a permission notice', async () => {
  setActivePinia(createPinia())
  env.fetch = async () => jsonResponse(USER)

  await router.push('/admin')

  assert.equal(router.currentRoute.value.path, '/')
  assert.ok(useNotice().text.length > 0)
})

// 管理员访问平台管理放行，并由 afterEach 更新页面标题
test('admin access to platform admin is allowed and afterEach updates the page title', async () => {
  setActivePinia(createPinia())
  env.fetch = async () => jsonResponse(ADMIN)

  await router.push('/admin')

  assert.equal(router.currentRoute.value.name, 'admin')
  assert.equal(env.document.title, '平台管理 · 问序')
})
