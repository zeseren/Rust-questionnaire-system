//! 只展示最新请求的结果，避免快速筛选时旧响应覆盖当前页面。
import { onBeforeUnmount, ref, shallowRef, watch, type WatchSource } from 'vue'
import { message } from './api'

export function useResource<T>(loader: () => Promise<T>, sources: WatchSource[] = []) {
  const data = shallowRef<T | null>(null)
  const error = ref('')
  const loading = ref(false)
  let sequence = 0
  async function refresh() {
    const request = ++sequence
    loading.value = true
    error.value = ''
    data.value = null
    try {
      const value = await loader()
      if (request === sequence) data.value = value
    } catch (reason) {
      if (request === sequence) error.value = message(reason)
    } finally {
      if (request === sequence) loading.value = false
    }
  }
  watch(sources, refresh, { immediate: true })
  onBeforeUnmount(() => {
    sequence += 1
  })
  return { data, error, loading, refresh }
}
