//! 会话仅保存在内存，只有外观偏好写入本地存储。
import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { api, ApiError, message } from './api'
import type { User } from './types'

export const useAuth = defineStore('auth', () => {
  const user = ref<User | null>(null)
  const initialized = ref(false)
  const expired = ref(false)
  const restoreError = ref('')
  let restoring: Promise<void> | null = null
  async function restore() {
    if (initialized.value) return
    if (restoring) return restoring
    restoring = (async () => {
      try {
        user.value = await api<User>('/api/auth/me')
        restoreError.value = ''
      } catch (error) {
        if (!(error instanceof ApiError && error.status === 401))
          restoreError.value = message(error)
      } finally {
        initialized.value = true
        restoring = null
      }
    })()
    return restoring
  }
  async function login(username: string, password: string) {
    user.value = await api<User>('/api/auth/login', 'POST', { username, password })
    expired.value = false
    initialized.value = true
    restoreError.value = ''
  }
  async function logout() {
    await api('/api/auth/logout', 'POST')
    user.value = null
    expired.value = false
  }
  return { user, initialized, expired, restoreError, restore, login, logout }
})

export type Theme = 'light' | 'dark' | 'system'
export const useAppearance = defineStore('appearance', () => {
  const preference = ref<Theme>('system')
  const systemDark = ref(matchMedia('(prefers-color-scheme: dark)').matches)
  try {
    const saved = localStorage.getItem('wenxu-theme')
    if (saved === 'light' || saved === 'dark' || saved === 'system') preference.value = saved
  } catch {}
  const dark = computed(
    () => preference.value === 'dark' || (preference.value === 'system' && systemDark.value),
  )
  function apply() {
    document.documentElement.dataset.theme = dark.value ? 'dark' : 'light'
    document
      .querySelector('meta[name="theme-color"]')
      ?.setAttribute('content', dark.value ? '#171717' : '#f7f7f7')
  }
  function setTheme(theme: Theme) {
    preference.value = theme
    apply()
    try {
      localStorage.setItem('wenxu-theme', theme)
    } catch {}
  }
  matchMedia('(prefers-color-scheme: dark)').addEventListener('change', (event) => {
    systemDark.value = event.matches
    apply()
  })
  apply()
  return { preference, dark, setTheme }
})

export const useNotice = defineStore('notice', () => {
  const text = ref('')
  let timer: ReturnType<typeof setTimeout>
  function show(value: string) {
    text.value = value
    clearTimeout(timer)
    timer = setTimeout(() => {
      text.value = ''
    }, 4500)
  }
  return { text, show }
})
