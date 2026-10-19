<!-- 修改密码成功后撤销前端登录态，与后端的全设备会话失效保持一致。 -->
<script setup lang="ts">
import { ref } from 'vue'
import { useRouter } from 'vue-router'
import { api, message } from '../api'
import { useAppearance, useAuth, useNotice, type Theme } from '../stores'
import Icon from '../components/Icon.vue'
import ErrorNotice from '../components/ErrorNotice.vue'
const auth = useAuth()
const appearance = useAppearance()
const notice = useNotice()
const router = useRouter()
const oldPassword = ref('')
const newPassword = ref('')
const confirmation = ref('')
const error = ref('')
const busy = ref(false)
const themes: { value: Theme; label: string; icon: string }[] = [
  { value: 'light', label: '浅色', icon: 'sun' },
  { value: 'dark', label: '深色', icon: 'moon' },
  { value: 'system', label: '跟随系统', icon: 'desktop' },
]
async function changePassword() {
  error.value = ''
  if ([...newPassword.value].length < 8 || [...newPassword.value].length > 128) {
    error.value = '新密码长度需为 8–128 个字符。'
    return
  }
  if (newPassword.value !== confirmation.value) {
    error.value = '两次输入的新密码不一致。'
    return
  }
  busy.value = true
  try {
    await api('/api/auth/password', 'PUT', {
      old_password: oldPassword.value,
      new_password: newPassword.value,
    })
    auth.user = null
    auth.expired = false
    oldPassword.value = ''
    newPassword.value = ''
    confirmation.value = ''
    notice.show('密码已修改，所有设备需重新登录。')
    await router.replace('/login')
  } catch (reason) {
    error.value = message(reason)
  } finally {
    busy.value = false
  }
}
</script>
<template>
  <div class="page-heading">
    <div>
      <p class="eyebrow">ACCOUNT &amp; APPEARANCE</p>
      <h1>账户设置</h1>
      <p class="heading-note">让工作区更适合你。</p>
    </div>
  </div>
  <div class="settings-layout">
    <section class="panel">
      <div class="panel-header"><h2>个人账户</h2></div>
      <div class="account-grid">
        <div>
          <span class="hint">用户名</span>
          <strong>{{ auth.user?.username }}</strong>
        </div>
        <div>
          <span class="hint">账户角色</span>
          <strong>{{ auth.user?.role === 'admin' ? '平台管理员' : '普通用户' }}</strong>
        </div>
      </div>
    </section>
    <section class="panel">
      <h2>外观</h2>
      <p class="subtle mt">温润纸本与松烟墨排版，让问卷编纂与阅读保持专注。</p>
      <div class="settings-themes" role="group" aria-label="选择外观主题">
        <button
          v-for="theme in themes"
          :key="theme.value"
          class="button"
          :aria-pressed="appearance.preference === theme.value"
          @click="appearance.setTheme(theme.value)"
        >
          <Icon :name="theme.icon" />
          {{ theme.label }}
        </button>
      </div>
    </section>
    <section class="panel">
      <h2>修改密码</h2>
      <p class="hint mt mb">修改成功后，所有设备（包括当前设备）都需要重新登录。</p>
      <form class="form-stack" @submit.prevent="changePassword">
        <ErrorNotice :message="error" />
        <label class="field">
          当前密码
          <input
            v-model="oldPassword"
            type="password"
            autocomplete="current-password"
            required
            :disabled="busy"
          />
        </label>
        <div class="field-row">
          <label class="field">
            新密码
            <input
              v-model="newPassword"
              type="password"
              autocomplete="new-password"
              required
              :disabled="busy"
            />
          </label>
          <label class="field">
            确认新密码
            <input
              v-model="confirmation"
              type="password"
              autocomplete="new-password"
              required
              :disabled="busy"
            />
          </label>
        </div>
        <div>
          <button class="button button-primary" :disabled="busy">
            {{ busy ? '正在修改…' : '更新密码' }}
          </button>
        </div>
      </form>
    </section>
  </div>
</template>
