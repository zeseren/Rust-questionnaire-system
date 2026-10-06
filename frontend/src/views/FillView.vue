<!-- 提交结果未知时冻结原载荷与幂等标识，重试不会悄悄创建第二次提交。 -->
<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, shallowRef } from 'vue'
import { onBeforeRouteLeave, RouterLink, useRoute } from 'vue-router'
import { api, ApiError, copy, message } from '../api'
import { buildAnswers, isAnswered, type Inputs } from '../answers'
import { dateTime, shareLink } from '../format'
import { useAuth } from '../stores'
import type { PublicSurvey, Receipt, Submission, User } from '../types'
import SurveyFields from '../components/SurveyFields.vue'
import AuthForm from '../components/AuthForm.vue'
import Modal from '../components/Modal.vue'
import Icon from '../components/Icon.vue'
import ErrorNotice from '../components/ErrorNotice.vue'
import LoadingState from '../components/LoadingState.vue'
const route = useRoute()
const auth = useAuth()
const id = String(route.params.id)
const survey = shallowRef<PublicSurvey | null>(null)
const receipt = shallowRef<Receipt | null>(null)
const values = ref<Inputs>({})
const errors = ref<Record<string, string>>({})
const error = ref('')
const loading = ref(true)
const busy = ref(false)
const loginNeeded = ref(false)
const uncertain = ref(false)
const versionChanged = ref(false)
const pending = shallowRef<Submission | null>(null)
const account = ref<{ id: string; username: string } | null>(null)
const answered = computed(() => Object.values(values.value).filter(isAnswered).length)
async function load() {
  loading.value = true
  error.value = ''
  try {
    survey.value = await api<PublicSurvey>(`/api/public/surveys/${id}`)
  } catch (reason) {
    if (reason instanceof ApiError && reason.status === 401) loginNeeded.value = true
    else error.value = message(reason)
  } finally {
    loading.value = false
  }
}
async function loginSuccess() {
  if (account.value && auth.user?.id !== account.value.id) {
    error.value = '请使用本次提交时的同一账号重新登录，当前输入仍然保留。'
    return
  }
  loginNeeded.value = false
  error.value = ''
  if (!survey.value) await load()
}
async function focusError() {
  const first = survey.value?.document.questions.find((question) => errors.value[question.id])
  if (first) {
    await nextTick()
    document.getElementById(`answer-${first.id}`)?.focus()
  }
}
async function submit() {
  if (
    !survey.value ||
    survey.value.document.logic ||
    busy.value ||
    receipt.value ||
    versionChanged.value
  )
    return
  error.value = ''
  if (!pending.value) {
    const result = buildAnswers(survey.value.document, values.value)
    errors.value = result.errors
    if (Object.keys(result.errors).length) {
      error.value = '请检查标记的问题后再提交。'
      await focusError()
      return
    }
    pending.value = {
      version_id: survey.value.version_id,
      idempotency_key: crypto.randomUUID(),
      answers: copy(result.answers),
    }
  }
  busy.value = true
  try {
    if (survey.value.mode === 'login_once') {
      const current = await api<User>('/api/auth/me')
      auth.user = current
      if (account.value && current.id !== account.value.id) {
        loginNeeded.value = true
        throw new ApiError(401, 'account_changed', '请使用本次提交时的同一账号重试。')
      }
      account.value = { id: current.id, username: current.username }
    }
    const result = await api<Receipt>(`/api/public/surveys/${id}/responses`, 'POST', pending.value)
    if (result.status !== 'submitted')
      throw new ApiError(0, 'invalid_response', '尚未收到完整回执，请重试原提交。')
    receipt.value = result
    pending.value = null
    uncertain.value = false
    window.scrollTo({ top: 0 })
  } catch (reason) {
    error.value = message(reason)
    if (reason instanceof ApiError) {
      if (reason.question_id) {
        errors.value = { [reason.question_id]: reason.message }
        await focusError()
      }
      if (reason.status === 401) loginNeeded.value = true
      if (reason.code === 'version_changed') versionChanged.value = true
      uncertain.value =
        reason.status === 0 ||
        reason.status >= 500 ||
        reason.status === 401 ||
        reason.code === 'idempotency_conflict'
    } else uncertain.value = true
    if (!uncertain.value) {
      pending.value = null
      account.value = null
    }
  } finally {
    busy.value = false
  }
}
function leave(event: BeforeUnloadEvent) {
  if (!receipt.value && (answered.value > 0 || uncertain.value)) {
    event.preventDefault()
    event.returnValue = ''
  }
}
onBeforeRouteLeave(
  () =>
    !!receipt.value ||
    (!answered.value && !uncertain.value) ||
    window.confirm('离开后当前回答不会保存。确定离开吗？'),
)
onMounted(() => {
  load()
  window.addEventListener('beforeunload', leave)
})
onBeforeUnmount(() => window.removeEventListener('beforeunload', leave))
</script>
<template>
  <div class="public-container">
    <LoadingState v-if="loading" />
    <section v-else-if="receipt && survey" class="panel receipt-panel">
      <span class="success-mark"><Icon name="check" /></span>
      <h1>回答已收到</h1>
      <p class="plain-text subtle mt">{{ survey.document.thank_you || '感谢你的认真回答。' }}</p>
      <code class="receipt">回执编号：{{ receipt.receipt_id }}</code>
      <p class="hint mt">你的回答已成功提交，可以关闭此页面。</p>
    </section>
    <template v-else>
      <ErrorNotice :message="error" :retry="!survey && !loginNeeded" @retry="load" />
      <div v-if="!survey && loginNeeded" class="panel">
        <h1>请先登录</h1>
        <p class="subtle mt mb">这份问卷需要登录后填写，每个账号限交一次。</p>
        <AuthForm @success="loginSuccess" />
        <p class="auth-switch">
          还没有账号？
          <RouterLink :to="{ name: 'register', query: { redirect: `/s/${id}` } }">
            创建账号
          </RouterLink>
        </p>
      </div>
      <section v-if="survey?.document.logic" class="panel">
        <h1>{{ survey.document.title }}</h1>
        <p class="notice mt">本问卷暂未开放网页填写，请联系发布者。</p>
      </section>
      <form v-if="survey && !survey.document.logic" novalidate @submit.prevent="submit">
        <section class="panel public-survey-header">
          <p class="eyebrow">
            {{ survey.mode === 'anonymous' ? '匿名填写' : '登录限次' }} · V{{
              survey.version_number
            }}
          </p>
          <h1>{{ survey.document.title }}</h1>
          <p class="plain-text subtle">{{ survey.document.description }}</p>
          <div class="mt">
            <strong class="hint">数据收集目的</strong>
            <p class="plain-text subtle">{{ survey.document.purpose }}</p>
          </div>
          <p v-if="survey.deadline" class="hint mt">
            截止时间：{{ dateTime(survey.deadline) }}（北京时间）
          </p>
          <p class="hint mt">{{ survey.privacy.mode_notice }}</p>
          <details class="privacy-details">
            <summary>数据可见范围与填写须知</summary>
            <p class="hint">{{ survey.privacy.data_visibility }}</p>
            <p class="hint">{{ survey.privacy.sensitive_data_notice }}</p>
            <p class="hint">{{ survey.privacy.public_link_notice }}</p>
          </details>
        </section>
        <div v-if="versionChanged" class="notice mt">
          <div>
            问卷已经更新，当前输入已保留供核对。
            <a :href="shareLink(id)" target="_blank" rel="noopener" class="accent">
              在新窗口打开最新题面
            </a>
            ，请按新题面重新填写。
          </div>
        </div>
        <div v-if="uncertain && !loginNeeded" class="notice mt">
          尚未确认原提交结果。当前回答已暂时锁定，点击“重试原提交”获取结果，避免重复提交。
        </div>
        <SurveyFields
          v-model="values"
          :document="survey.document"
          :errors="errors"
          :disabled="busy || uncertain || versionChanged"
        />
        <div class="fill-footer">
          <span class="hint">
            已回答 {{ answered }} / {{ survey.document.questions.length }} 题 ·
            标注“必填”的题目必须回答
          </span>
          <button class="button button-primary" :disabled="busy || versionChanged || loginNeeded">
            {{ busy ? '正在提交…' : uncertain ? '重试原提交' : '提交答卷' }}
          </button>
        </div>
      </form>
    </template>
  </div>
  <Modal v-if="loginNeeded && survey" title="请重新登录后继续" :dismissible="false">
    <p class="subtle mb">
      当前回答仍然保留。{{ account ? '请使用提交时的同一账号登录。' : '请登录后重试提交。' }}
    </p>
    <ErrorNotice :message="error" />
    <AuthForm
      :initial-username="account?.username || auth.user?.username"
      @success="loginSuccess"
    />
  </Modal>
</template>
