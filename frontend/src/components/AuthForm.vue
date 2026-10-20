<!-- [入口在哪] 登录/注册页与会话过期登录弹窗内渲染本表单 -->
<!-- [用户在第 1 步后悔了怎么删/退] 注册成功但自动登录失败时，底栏左侧常驻“更换账号重新注册”，可清空输入回到注册初态 -->
<!-- [主提交通道在哪] 表单底栏右侧的唯一主按钮“登录/创建账号并登录” -->
<!-- 密码只用于当前认证请求，失败时不写入日志或浏览器存储。 -->
<script setup lang="ts">
import { ref } from 'vue'
import { api, message } from '../api'
import { useAuth } from '../stores'
import ErrorNotice from './ErrorNotice.vue'
const props = withDefaults(
  defineProps<{ mode?: 'login' | 'register'; initialUsername?: string }>(),
  { mode: 'login', initialUsername: '' },
)
const emit = defineEmits<{ success: [] }>()
const auth = useAuth()
const username = ref(props.initialUsername)
const password = ref('')
const confirmation = ref('')
const error = ref('')
const busy = ref(false)
const registered = ref(false)
const visible = ref(false)
async function submit() {
  error.value = ''
  if (!/^[A-Za-z0-9_]{3,32}$/.test(username.value)) {
    error.value = '用户名需为 3–32 个字母、数字或下划线。'
    return
  }
  if ([...password.value].length < 8 || [...password.value].length > 128) {
    error.value = '密码长度需为 8–128 个字符。'
    return
  }
  if (props.mode === 'register' && !registered.value && password.value !== confirmation.value) {
    error.value = '两次输入的密码不一致。'
    return
  }
  busy.value = true
  try {
    if (props.mode === 'register' && !registered.value) {
      await api('/api/auth/register', 'POST', {
        username: username.value,
        password: password.value,
      })
      registered.value = true
    }
    await auth.login(username.value, password.value)
    password.value = ''
    confirmation.value = ''
    emit('success')
  } catch (reason) {
    error.value = message(reason)
  } finally {
    busy.value = false
  }
}
function switchAccount() {
  registered.value = false
  error.value = ''
  password.value = ''
  confirmation.value = ''
}
</script>
<template>
  <form class="form-stack" @submit.prevent="submit">
    <p v-if="registered" class="notice">账号已创建，请登录后继续。</p>
    <ErrorNotice :message="error" />
    <label class="field">
      用户名
      <input
        v-model="username"
        name="username"
        autocomplete="username"
        autocapitalize="none"
        spellcheck="false"
        required
        :disabled="busy || registered"
        placeholder="请输入用户名"
      />
    </label>
    <label class="field">
      密码
      <span class="password-input">
        <input
          v-model="password"
          name="password"
          :type="visible ? 'text' : 'password'"
          :autocomplete="mode === 'register' && !registered ? 'new-password' : 'current-password'"
          required
          :disabled="busy"
          placeholder="请输入密码"
        />
        <button
          class="text-button"
          type="button"
          :aria-pressed="visible"
          @click="visible = !visible"
        >
          {{ visible ? '隐藏' : '显示' }}
        </button>
      </span>
    </label>
    <label v-if="mode === 'register' && !registered" class="field">
      确认密码
      <input
        v-model="confirmation"
        name="confirmation"
        type="password"
        autocomplete="new-password"
        required
        :disabled="busy"
        placeholder="再次输入密码"
      />
    </label>
    <p v-if="mode === 'register' && !registered" class="hint">
      用户名支持字母、数字和下划线；密码至少 8 个字符。
    </p>
    <div class="actions" :class="registered ? 'between' : 'end'">
      <button
        v-if="registered"
        type="button"
        class="text-button danger-text"
        @click="switchAccount"
      >
        更换账号重新注册
      </button>
      <button
        class="button button-primary"
        :class="{ 'button-block': !registered }"
        :disabled="busy"
      >
        {{ busy ? '请稍候…' : mode === 'register' && !registered ? '创建账号并登录' : '登录' }}
      </button>
    </div>
  </form>
</template>
