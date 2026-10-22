//! 前端单元测试的浏览器环境桩：只实现被测模块真正读取的 API，安装与卸载严格成对，
//! 避免多个测试文件共享全局状态。默认通过 `t.after` 自动清理，也可显式调用 `restore()`。
//!
//! 覆盖范围：
//! - `window` / `document`（标题、`documentElement.dataset`、`theme-color` meta、`<a>` 元素）与 `session-expired`
//!   事件派发所需的 `EventTarget`；
//! - `localStorage`（内存 Map，可注入读写抛错以验证容错分支）；
//! - `matchMedia`（可手动切换系统深色偏好并触发 `change` 监听器）；
//! - `location` / `history`（供 `vue-router` 的 `createWebHistory` 初始化）；
//! - `fetch` 与 `URL.createObjectURL` / `URL.revokeObjectURL`（供 `api.ts` 的传输层）；
//! - `window.setTimeout` / `window.clearTimeout`：默认委托给 `globalThis`，`timers: 'manual'`
//!   时改为手动调度，便于在 30 秒截止与下载清理定时器上同步断言。

const GLOBAL_KEYS = [
  'window',
  'document',
  'localStorage',
  'matchMedia',
  'location',
  'history',
  'fetch',
]

/** 通用事件目标桩：只保存函数监听器，派发时按注册顺序同步调用。 */
function createEventTarget() {
  const listeners = new Map()
  return {
    addEventListener(type, listener) {
      if (typeof listener !== 'function') return
      if (!listeners.has(type)) listeners.set(type, new Set())
      listeners.get(type).add(listener)
    },
    removeEventListener(type, listener) {
      listeners.get(type)?.delete(listener)
    },
    dispatchEvent(event) {
      for (const listener of [...(listeners.get(event.type) ?? [])]) listener(event)
      return true
    },
  }
}

/** 内存本地存储；`failure.read` / `failure.write` 打开时对应操作抛错。 */
function createStorage(failure) {
  const entries = new Map()
  return {
    get length() {
      return entries.size
    },
    key(index) {
      return [...entries.keys()][index] ?? null
    },
    getItem(key) {
      if (failure.read) throw new Error('本地存储不可读取')
      return entries.get(key) ?? null
    },
    setItem(key, value) {
      if (failure.write) throw new Error('本地存储不可写入')
      entries.set(key, String(value))
    },
    removeItem(key) {
      if (failure.write) throw new Error('本地存储不可写入')
      entries.delete(key)
    },
    clear() {
      entries.clear()
    },
  }
}

/** 定时器桩：manual 模式返回自增 id 并记录待执行回调。 */
function createTimers(mode) {
  const manual = mode === 'manual'
  const pending = []
  const cleared = []
  let nextId = 1
  return {
    pending,
    cleared,
    setTimeout(callback, delay = 0, ...args) {
      if (!manual) return globalThis.setTimeout(callback, delay, ...args)
      const entry = { id: nextId++, callback, delay, args }
      pending.push(entry)
      return entry.id
    },
    clearTimeout(id) {
      if (!manual) {
        globalThis.clearTimeout(id)
        return
      }
      const index = pending.findIndex((entry) => entry.id === id)
      if (index < 0) return
      pending.splice(index, 1)
      cleared.push(id)
    },
    /** 同步执行当前全部待执行回调；回调中再次排定的定时器留在队列中。 */
    run() {
      for (const entry of pending.splice(0)) entry.callback(...entry.args)
    },
  }
}

/** 深色偏好查询的匹配器；其余媒体查询一律不匹配。 */
const isDarkQuery = (query) => String(query).includes('prefers-color-scheme: dark')

/** matchMedia 桩：保留全部 MediaQueryList，便于统一切换系统偏好。 */
function createMatchMedia(initialDark) {
  let systemDark = Boolean(initialDark)
  const lists = new Set()
  const match = (query) => {
    const listeners = new Set()
    const list = {
      media: String(query),
      matches: isDarkQuery(query) ? systemDark : false,
      addEventListener(type, listener) {
        if (type === 'change' && typeof listener === 'function') listeners.add(listener)
      },
      removeEventListener(type, listener) {
        if (type === 'change') listeners.delete(listener)
      },
      dispatch() {
        for (const listener of [...listeners])
          listener({ matches: list.matches, media: list.media })
      },
    }
    lists.add(list)
    return list
  }
  /** 切换系统深色偏好，并向所有深色查询监听器派发 `change`。 */
  const setSystemDark = (value) => {
    systemDark = Boolean(value)
    for (const list of lists) {
      if (!isDarkQuery(list.media)) continue
      list.matches = systemDark
      list.dispatch()
    }
  }
  return { match, setSystemDark }
}

/**
 * 安装浏览器环境桩。
 *
 * @param {object} [t] `node:test` 的测试或钩子上下文；提供时通过 `t.after(restore)` 自动卸载。
 * @param {object} [options]
 * @param {'manual'} [options.timers] 传入 `manual` 时由 `env.timers` 手动调度定时器。
 * @param {boolean|'read'|'write'|'all'} [options.storageThrows] 初始本地存储抛错模式。
 * @param {boolean} [options.systemDark] 初始系统深色偏好。
 * @param {string} [options.pathname] 初始 `location.pathname`。
 */
export function installBrowserEnv(t, options = {}) {
  /** 未安装桩时立即失败，避免测试误用真实网络。 */
  const unconfiguredFetch = async () => {
    throw new TypeError('测试未安装 fetch 桩')
  }

  const failure = {
    read: ['read', 'all', true].includes(options.storageThrows),
    write: ['write', 'all', true].includes(options.storageThrows),
  }
  const storage = createStorage(failure)
  const timers = createTimers(options.timers)
  const { match, setSystemDark } = createMatchMedia(options.systemDark)

  const location = {
    protocol: 'http:',
    host: 'localhost:5173',
    origin: 'http://localhost:5173',
    pathname: options.pathname ?? '/',
    search: '',
    hash: '',
    href: 'http://localhost:5173/',
  }
  location.href = `${location.origin}${location.pathname}`

  /** 按 vue-router 传入的地址更新桩地址；不执行真实导航。 */
  const navigate = (url) => {
    const target = new URL(String(url), location.origin)
    location.pathname = target.pathname
    location.search = target.search
    location.hash = target.hash
    location.href = target.href
  }

  const history = {
    state: null,
    length: 1,
    goCalls: [],
    pushState(state, _title, url) {
      this.state = state
      this.length += 1
      navigate(url)
    },
    replaceState(state, _title, url) {
      this.state = state
      navigate(url)
    },
    go(delta) {
      this.goCalls.push(delta)
    },
    back() {
      this.go(-1)
    },
    forward() {
      this.go(1)
    },
  }

  const meta = {
    attributes: new Map(),
    getAttribute(name) {
      return this.attributes.get(name) ?? null
    },
    setAttribute(name, value) {
      this.attributes.set(name, String(value))
    },
  }

  const anchors = []
  const document = {
    ...createEventTarget(),
    title: '',
    visibilityState: 'visible',
    documentElement: { dataset: {}, style: {} },
    querySelector(selector) {
      return selector === 'meta[name="theme-color"]' ? meta : null
    },
    createElement(tagName) {
      const element = {
        tagName: String(tagName).toUpperCase(),
        attributes: new Map(),
        href: '',
        download: '',
        clicks: 0,
        click() {
          this.clicks += 1
        },
        setAttribute(name, value) {
          this.attributes.set(name, String(value))
        },
        remove() {},
      }
      if (element.tagName === 'A') anchors.push(element)
      return element
    },
  }

  const objectUrls = { created: [], revoked: [], blobs: [] }
  let objectUrlSequence = 0
  const urlDescriptors = {
    createObjectURL: Object.getOwnPropertyDescriptor(URL, 'createObjectURL'),
    revokeObjectURL: Object.getOwnPropertyDescriptor(URL, 'revokeObjectURL'),
  }
  Object.defineProperty(URL, 'createObjectURL', {
    configurable: true,
    writable: true,
    value(blob) {
      const url = `blob:test/${++objectUrlSequence}`
      objectUrls.created.push(url)
      objectUrls.blobs.push(blob)
      return url
    },
  })
  Object.defineProperty(URL, 'revokeObjectURL', {
    configurable: true,
    writable: true,
    value(url) {
      objectUrls.revoked.push(url)
    },
  })

  const window = {
    ...createEventTarget(),
    document,
    location,
    history,
    localStorage: storage,
    matchMedia: match,
    setTimeout: timers.setTimeout,
    clearTimeout: timers.clearTimeout,
    getComputedStyle: () => ({ getPropertyValue: () => '' }),
    scrollTo() {},
  }
  window.window = window
  window.self = window

  const env = {
    window,
    document,
    location,
    history,
    localStorage: storage,
    meta,
    anchors,
    timers,
    objectUrls,
    /** 本地存储抛错开关，可随时翻转以覆盖读取与写入两条容错分支。 */
    failStorage: failure,
    fetch: unconfiguredFetch,
    setSystemDark,
    restore,
  }
  // 延迟读取 env.fetch，测试可随时替换桩实现而不必重新安装环境。
  window.fetch = (...args) => env.fetch(...args)

  const saved = new Map()
  for (const key of GLOBAL_KEYS) saved.set(key, Object.getOwnPropertyDescriptor(globalThis, key))
  const globals = {
    window,
    document,
    localStorage: storage,
    matchMedia: match,
    location,
    history,
    fetch: window.fetch,
  }
  for (const [key, value] of Object.entries(globals))
    Object.defineProperty(globalThis, key, {
      configurable: true,
      writable: true,
      enumerable: true,
      value,
    })

  let restored = false
  function restore() {
    if (restored) return
    restored = true
    for (const [key, descriptor] of saved) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor)
      else delete globalThis[key]
    }
    for (const [name, descriptor] of Object.entries(urlDescriptors)) {
      if (descriptor) Object.defineProperty(URL, name, descriptor)
      else delete URL[name]
    }
  }

  if (t && typeof t.after === 'function') t.after(restore)
  return env
}
