//! `useResource` 的加载契约：创建即加载、失败文案提取、刷新清空旧值，
//! 以及“只展示最新请求结果”的并发竞态保护。
import assert from 'node:assert/strict'
import test from 'node:test'
import { createRenderer, defineComponent, nextTick, ref } from 'vue'
import { useResource } from '../../src/resource.ts'

/** 等待微任务（含 loader 的 await 续体）与 Vue 调度队列全部落定。 */
const settle = async () => {
  await nextTick()
  await new Promise((resolve) => setTimeout(resolve, 0))
}

function deferred() {
  let resolve, reject
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}

/** 组件外调用 `useResource` 会触发 Vue 的 “no active component instance” 告警，与断言无关。 */
const silenceVueWarnings = (t) => t.mock.method(console, 'warn', () => {})

test('创建即自动加载，成功时写入数据并结束加载态', async (t) => {
  silenceVueWarnings(t)
  const load = deferred()
  const resource = useResource(() => load.promise)

  // watch 的 immediate 语义：创建瞬间已进入加载态，避免首屏出现“空数据且未加载”。
  assert.equal(resource.loading.value, true)
  assert.equal(resource.data.value, null)
  assert.equal(resource.error.value, '')

  load.resolve({ id: '问卷' })
  await load.promise
  await settle()

  assert.deepEqual(resource.data.value, { id: '问卷' })
  assert.equal(resource.error.value, '')
  assert.equal(resource.loading.value, false)
})

test('加载失败提取 Error.message 并清空旧数据与加载态', async (t) => {
  silenceVueWarnings(t)
  const resource = useResource(async () => {
    throw new Error('接口读取失败')
  })
  await settle()

  assert.equal(resource.data.value, null)
  assert.equal(resource.error.value, '接口读取失败')
  assert.equal(resource.loading.value, false)
})

test('刷新成功后清空上一次的错误文案', async (t) => {
  silenceVueWarnings(t)
  let attempt = 0
  const resource = useResource(async () => {
    attempt += 1
    if (attempt === 1) throw new Error('第一次失败')
    return { id: attempt }
  })

  await settle()
  assert.equal(resource.error.value, '第一次失败')

  await resource.refresh()
  await settle()

  assert.deepEqual(resource.data.value, { id: 2 })
  assert.equal(resource.error.value, '')
  assert.equal(resource.loading.value, false)
})

test('慢请求后完成的过期成功结果不会覆盖最新数据', async (t) => {
  silenceVueWarnings(t)
  const slow = deferred()
  const fast = deferred()
  let calls = 0
  const resource = useResource(() => {
    calls += 1
    return calls === 1 ? slow.promise : fast.promise
  })

  resource.refresh()
  assert.equal(calls, 2)
  // 刷新立即清空旧数据，避免旧内容与新加载态并存。
  assert.equal(resource.data.value, null)
  assert.equal(resource.loading.value, true)

  fast.resolve({ title: '最新结果' })
  await settle()
  assert.deepEqual(resource.data.value, { title: '最新结果' })
  assert.equal(resource.loading.value, false)

  slow.resolve({ title: '过期结果' })
  await settle()
  assert.deepEqual(resource.data.value, { title: '最新结果' })
  assert.equal(resource.loading.value, false)
  assert.equal(resource.error.value, '')
})

test('过期请求的失败不会写入错误或提前结束加载态', async (t) => {
  silenceVueWarnings(t)
  const slow = deferred()
  const latest = deferred()
  let calls = 0
  const resource = useResource(() => {
    calls += 1
    return calls === 1 ? slow.promise : latest.promise
  })

  resource.refresh()
  latest.resolve('最新内容')
  await settle()
  assert.equal(resource.data.value, '最新内容')

  slow.reject(new Error('过期请求失败'))
  await settle()
  assert.equal(resource.error.value, '')
  assert.equal(resource.data.value, '最新内容')
  assert.equal(resource.loading.value, false)
})

test('响应式来源变化会重新加载并采用新来源的结果', async (t) => {
  silenceVueWarnings(t)
  const status = ref('全部')
  const requested = []
  const resource = useResource(async () => {
    requested.push(status.value)
    return `状态：${status.value}`
  }, [() => status.value])
  await settle()
  assert.deepEqual(requested, ['全部'])
  assert.equal(resource.data.value, '状态：全部')

  status.value = '已归档'
  await settle()

  assert.deepEqual(requested, ['全部', '已归档'])
  assert.equal(resource.data.value, '状态：已归档')
  assert.equal(resource.error.value, '')
})

test('组件卸载会作废挂起请求，卸载后完成的数据不再写入', async () => {
  const load = deferred()
  let resource
  const Root = defineComponent({
    setup() {
      resource = useResource(() => load.promise)
      return () => null
    },
  })
  // 卸载断言只需生命周期，不需要真实 DOM：用空操作宿主渲染器提供组件实例。
  const renderer = createRenderer({
    patchProp() {},
    insert() {},
    remove() {},
    createElement: () => ({}),
    createText: () => ({}),
    createComment: () => ({}),
    setText() {},
    setElementText() {},
    parentNode: () => null,
    nextSibling: () => null,
    querySelector: () => null,
    setScopeId() {},
    insertStaticContent: () => [{}, {}],
  })
  const app = renderer.createApp(Root)
  app.mount({})

  assert.equal(resource.loading.value, true)
  app.unmount()

  load.resolve({ id: '卸载后完成' })
  await settle()
  assert.equal(resource.data.value, null)
  assert.equal(resource.error.value, '')
})
