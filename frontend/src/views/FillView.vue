<!-- [入口在哪] 公开填写链接 /s/:id；先展示免登录的问卷说明与隐私提示，再按模式加载题面。 -->
<!-- [用户在第 1 步后悔了怎么删/退] 底部固定栏左侧常驻「清空重填」；不可关闭的重新登录弹窗也提供「先返回首页」。 -->
<!-- [主提交通道在哪] 底部固定栏右侧唯一主按钮：「下一步/提交答卷/重试原提交/加载最新题面重填」。 -->
<!-- 提交结果未知时冻结原载荷与幂等标识，重试不会悄悄创建第二次提交；未提交的填写进度按版本暂存在本地。 -->
<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, shallowRef } from 'vue'
import { onBeforeRouteLeave, RouterLink, useRoute } from 'vue-router'
import { api, ApiError, message } from '../api'
import { buildAnswers, isAnswered, type Inputs } from '../answers'
import { dateTime, shareLink } from '../format'
import { decode } from '../json'
import { useAuth } from '../stores'
import { newId } from '../uuid'
import type {
  Answer,
  FlowEvent,
  PublicSurvey,
  PublicSurveyMeta,
  Receipt,
  Submission,
} from '../types'
import SurveyFields from '../components/SurveyFields.vue'
import FlowFields from '../components/FlowFields.vue'
import AuthForm from '../components/AuthForm.vue'
import Modal from '../components/Modal.vue'
import Icon from '../components/Icon.vue'
import ErrorNotice from '../components/ErrorNotice.vue'
import LoadingState from '../components/LoadingState.vue'
const route = useRoute()
const auth = useAuth()
const id = String(route.params.id)
const STORAGE_PREFIX = 'wenxu:fill-draft:'
/** 本地暂存按问卷与发布版本隔离，换版本后不会把旧输入带到新题面。 */
interface FillDraft {
  values?: Inputs
  events?: FlowEvent[]
}
const meta = shallowRef<PublicSurveyMeta | null>(null)
const survey = shallowRef<PublicSurvey | null>(null)
const receipt = shallowRef<Receipt | null>(null)
const receiptPanel = ref<HTMLElement | null>(null)
const values = ref<Inputs>({})
const flowDraft = ref<FlowEvent[]>([])
const errors = ref<Record<string, string>>({})
const error = ref('')
const loading = ref(true)
const busy = ref(false)
const loginNeeded = ref(false)
const uncertain = ref(false)
const versionChanged = ref(false)
const restored = ref(false)
const pending = shallowRef<Submission | null>(null)
const flowFields = ref<InstanceType<typeof FlowFields>>()
const flowAnswered = ref(0)
/** 「登录限次」但当前没有会话：先展示上下文与登录引导，不直接撞 401。 */
const loginRequired = computed(() => meta.value?.mode === 'login_once' && !auth.user)
const draftKey = computed(() =>
  survey.value ? `${STORAGE_PREFIX}${id}:${survey.value.version_id}` : '',
)
function flowChanged(count: number) {
  flowAnswered.value = count
  errors.value = {}
  scheduleSave()
}
const answered = computed(() =>
  survey.value?.document.logic
    ? flowAnswered.value
    : Object.values(values.value).filter(isAnswered).length,
)
function readDraft(key: string): FillDraft | null {
  try {
    const raw = localStorage.getItem(key)
    if (!raw) return null
    const draft = decode<FillDraft | null>(raw)
    if (!draft || typeof draft !== 'object') return null
    return draft
  } catch {
    return null
  }
}
/** 立即写入当前进度；提交成功后回执已存在，不再改写本地暂存。 */
function writeDraft() {
  if (!draftKey.value || receipt.value) return
  const draft: FillDraft = {}
  if (survey.value?.document.logic) draft.events = flowFields.value?.events() ?? []
  else draft.values = values.value
  try {
    localStorage.setItem(draftKey.value, JSON.stringify(draft))
  } catch {
    // 隐私模式等存储不可用时静默跳过，填写本身不受影响。
  }
}
let saveTimer: ReturnType<typeof setTimeout> | undefined
function scheduleSave() {
  clearTimeout(saveTimer)
  saveTimer = setTimeout(writeDraft, 400)
}
function clearDraft() {
  clearTimeout(saveTimer)
  saveTimer = undefined
  if (!draftKey.value) return
  try {
    localStorage.removeItem(draftKey.value)
  } catch {}
}
/** 流程页「清空重填」：同时丢弃已回放的进度，避免下次初始化又被恢复。 */
function resetFlowDraft() {
  flowDraft.value = []
  clearDraft()
}
function applyStoredDraft(result: PublicSurvey) {
  values.value = {}
  flowDraft.value = []
  restored.value = false
  const draft = readDraft(`${STORAGE_PREFIX}${id}:${result.version_id}`)
  if (!draft) return
  if (result.document.logic) {
    if (!Array.isArray(draft.events) || !draft.events.length) return
    flowDraft.value = draft.events
  } else {
    if (!draft.values || typeof draft.values !== 'object') return
    values.value = draft.values
  }
  restored.value = true
}
async function load() {
  loading.value = true
  error.value = ''
  loginNeeded.value = false
  survey.value = null
  receipt.value = null
  try {
    meta.value = await api<PublicSurveyMeta>(`/api/public/surveys/${id}/meta`)
  } catch (reason) {
    error.value = message(reason)
    loading.value = false
    return
  }
  loading.value = false
  if (!loginRequired.value) await loadSurvey()
}
async function loadSurvey() {
  loading.value = true
  error.value = ''
  try {
    const result = await api<PublicSurvey>(`/api/public/surveys/${id}`)
    survey.value = result
    versionChanged.value = false
    pending.value = null
    receipt.value = null
    errors.value = {}
    flowAnswered.value = 0
    applyStoredDraft(result)
  } catch (reason) {
    if (reason instanceof ApiError && reason.status === 401) loginNeeded.value = true
    else error.value = message(reason)
  } finally {
    loading.value = false
  }
}
function resetFill() {
  if (busy.value || !answered.value) return
  if (!window.confirm('确定清空当前已填写的回答吗？')) return
  error.value = ''
  values.value = {}
  errors.value = {}
  flowAnswered.value = 0
  flowFields.value?.reset()
  restored.value = false
  clearDraft()
}
async function loginSuccess() {
  loginNeeded.value = false
  error.value = ''
  if (!survey.value) await loadSurvey()
}
async function focusError() {
  const first = survey.value?.document.questions.find((question) => errors.value[question.id])
  if (first) {
    if (survey.value?.document.logic) {
      await flowFields.value?.focusQuestion(first.id)
      return
    }
    await nextTick()
    document.getElementById(`answer-${first.id}`)?.focus()
  }
}
async function submit(flowAnswers?: Answer[]) {
  if (!survey.value || busy.value || receipt.value || versionChanged.value) return
  error.value = ''
  if (!pending.value) {
    if (survey.value.document.logic) flowAnswers ??= flowFields.value?.submissionAnswers()
    if (survey.value.document.logic && !flowAnswers) return
    const result = flowAnswers
      ? { answers: flowAnswers, errors: {} }
      : buildAnswers(survey.value.document, values.value)
    errors.value = result.errors
    if (Object.keys(result.errors).length) {
      error.value = '请检查标记的问题后再提交。'
      await focusError()
      return
    }
    pending.value = {
      version_id: survey.value.version_id,
      // UUIDv7 同样满足后端对幂等秘密的版本校验。
      idempotency_key: newId(),
      answers: result.answers.map((answer) => ({
        ...answer,
        value: Array.isArray(answer.value) ? [...answer.value] : answer.value,
      })),
    }
  }
  // 登录态由提交接口鉴权；未登录时先弹出登录表单，不额外预请求账号信息。
  if (survey.value.mode === 'login_once' && !auth.user) {
    loginNeeded.value = true
    return
  }
  busy.value = true
  try {
    const result = await api<Receipt>(`/api/public/surveys/${id}/responses`, 'POST', pending.value)
    if (result.status !== 'submitted')
      throw new ApiError(0, 'invalid_response', '尚未收到完整回执，请重试原提交。')
    receipt.value = result
    pending.value = null
    uncertain.value = false
    restored.value = false
    clearDraft()
    window.scrollTo({ top: 0 })
    await nextTick()
    receiptPanel.value?.focus()
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
    if (!uncertain.value) pending.value = null
  } finally {
    busy.value = false
    await nextTick()
    if (!uncertain.value && !versionChanged.value) await focusError()
  }
}
function leave(event: BeforeUnloadEvent) {
  writeDraft()
  if (!receipt.value && (answered.value > 0 || uncertain.value)) {
    event.preventDefault()
    event.returnValue = ''
  }
}
onBeforeRouteLeave(() => {
  writeDraft()
  return (
    !!receipt.value ||
    (!answered.value && !uncertain.value) ||
    window.confirm('离开后可在同一版本继续填写，是否离开？')
  )
})
onMounted(() => {
  load()
  window.addEventListener('beforeunload', leave)
})
onBeforeUnmount(() => {
  clearTimeout(saveTimer)
  window.removeEventListener('beforeunload', leave)
})
</script>
<template>
  <div class="public-container">
    <LoadingState v-if="loading" />
    <section
      v-else-if="receipt && meta"
      ref="receiptPanel"
      class="panel receipt-panel"
      tabindex="-1"
      role="status"
      aria-live="polite"
    >
      <template v-if="receipt.outcome === 'screen_out'">
        <span class="success-mark"><Icon name="info" /></span>
        <h1>本次填写已结束</h1>
        <p class="plain-text subtle mt">
          根据你的回答，这份问卷进入了筛选结束分支。回执已记录，但这份回答不会计入答卷统计。
        </p>
        <code class="receipt">回执编号：{{ receipt.receipt_id }}</code>
        <p class="hint mt">无需其他操作，可以关闭此页面。</p>
      </template>
      <template v-else>
        <span class="success-mark"><Icon name="check" /></span>
        <h1>回答已收到</h1>
        <p class="plain-text subtle mt">
          {{ survey?.document.thank_you || '感谢你的认真回答。' }}
        </p>
        <code class="receipt">回执编号：{{ receipt.receipt_id }}</code>
        <p class="hint mt">你的回答已成功提交，可以关闭此页面。</p>
      </template>
      <div v-if="auth.user" class="actions mt">
        <RouterLink class="button" to="/">返回工作台</RouterLink>
      </div>
      <p v-else class="hint mt">可以关闭此页面。</p>
    </section>
    <template v-else>
      <ErrorNotice :message="error" :retry="!survey && !loginRequired" @retry="load" />
      <section v-if="meta" class="panel public-survey-header">
        <p class="eyebrow">
          {{ meta.mode === 'anonymous' ? '匿名填写' : '登录限次' }} · 填写前须知
        </p>
        <h1>{{ meta.title }}</h1>
        <p class="plain-text subtle">{{ meta.description }}</p>
        <div class="mt">
          <strong class="hint">数据收集目的</strong>
          <p class="plain-text subtle">{{ meta.purpose }}</p>
        </div>
        <p v-if="meta.deadline" class="hint mt">
          截止时间：{{ dateTime(meta.deadline) }}（北京时间）
        </p>
        <p class="hint mt">{{ meta.privacy.mode_notice }}</p>
        <details class="privacy-details">
          <summary>数据可见范围与填写须知</summary>
          <p class="hint">{{ meta.privacy.data_visibility }}</p>
          <p class="hint">{{ meta.privacy.sensitive_data_notice }}</p>
          <p class="hint">{{ meta.privacy.public_link_notice }}</p>
        </details>
      </section>
      <div v-if="loginRequired" class="panel">
        <h2>请先登录后填写</h2>
        <p class="subtle mt mb">
          这份问卷采用「登录限次」模式：需要登录，每个账号跨版本仅限一次，不属于完全匿名。登录后会自动加载题面。
        </p>
        <AuthForm @success="loginSuccess" />
        <p class="auth-switch">
          还没有账号？
          <RouterLink :to="{ name: 'register', query: { redirect: `/s/${id}` } }">
            创建账号
          </RouterLink>
        </p>
      </div>
      <form v-else-if="survey" novalidate @submit.prevent="submit()">
        <div v-if="restored" class="notice mt">
          <div class="grow">已恢复上次未提交的填写进度，可继续填写或清空重试。</div>
          <button type="button" class="text-button" @click="resetFill">清空重填</button>
        </div>
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
        <FlowFields
          v-if="survey.document.logic"
          ref="flowFields"
          :document="survey.document"
          :restore="flowDraft"
          :errors="errors"
          :disabled="busy || uncertain || versionChanged"
          :submit-disabled="loginNeeded"
          :hide-footer="uncertain || versionChanged"
          @change="flowChanged"
          @reset="resetFlowDraft"
          @submit="submit"
        />
        <SurveyFields
          v-else
          v-model="values"
          :document="survey.document"
          :errors="errors"
          :disabled="busy || uncertain || versionChanged"
          @change="scheduleSave()"
        />
        <div
          v-if="!survey.document.logic || uncertain || versionChanged"
          class="fill-footer actions between"
        >
          <div class="actions fill-footer-left">
            <template v-if="!uncertain && !versionChanged">
              <button
                type="button"
                class="text-button danger-text"
                :disabled="busy || !answered"
                @click="resetFill"
              >
                清空重填
              </button>
              <span class="hint">
                已回答 {{ answered }} / {{ survey.document.questions.length }} 题 ·
                标注“必填”的题目必须回答
              </span>
            </template>
            <a
              v-else-if="versionChanged"
              :href="shareLink(id)"
              target="_blank"
              rel="noopener"
              class="text-button accent"
            >
              打开新窗口核对
            </a>
            <span v-else class="hint">当前回答已锁定，请重试原提交以获取结果。</span>
          </div>
          <div class="actions fill-footer-right">
            <button
              v-if="versionChanged"
              type="button"
              class="button button-primary"
              :disabled="busy"
              @click="load"
            >
              加载最新题面重填
            </button>
            <button v-else class="button button-primary" :disabled="busy || loginNeeded">
              {{ busy ? '正在提交…' : uncertain ? '重试原提交' : '提交答卷' }}
            </button>
          </div>
        </div>
      </form>
      <LoadingState v-else-if="!error" />
    </template>
  </div>
  <Modal v-if="loginNeeded && meta" title="请重新登录后继续" :dismissible="false">
    <p class="subtle mb">当前回答仍然保留。请登录后重试提交。</p>
    <ErrorNotice :message="error" />
    <AuthForm :initial-username="auth.user?.username" @success="loginSuccess" />
    <div class="actions between mt">
      <RouterLink class="text-button" to="/">先返回首页</RouterLink>
    </div>
  </Modal>
</template>
