<!-- [入口在哪] 公开填写链接 /s/:id；先展示免登录的问卷说明与隐私提示，再按模式加载题面。 -->
<!-- [用户在第 1 步后悔了怎么删/退] 底部固定栏左侧常驻「清空重填」；不可关闭的重新登录弹窗也提供「先返回首页」。 -->
<!-- [主提交通道在哪] 底部固定栏右侧唯一主按钮：「下一步/提交答卷/重试原提交/加载最新题面重填」。 -->
<!-- 提交结果未知时冻结原载荷与幂等标识，重试不会悄悄创建第二次提交；未提交的填写进度按版本暂存在本地。 -->
<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, shallowRef } from 'vue'
import { onBeforeRouteLeave, RouterLink, useRoute } from 'vue-router'
import { api, ApiError, message } from '../api'
import { dateTime, shareLink } from '../format'
import { decode } from '../json'
import {
  ANONYMOUS_SCOPE,
  draftKeyFor,
  freezePending,
  pendingKeyFor,
  scopeFor,
} from '../pending-submission'
import type { FrozenPending } from '../pending-submission'
import { useAuth } from '../stores'
import { newId } from '../uuid'
import type { Answer, FlowEvent, PublicSurvey, PublicSurveyMeta, Receipt } from '../types'
import FlowFields from '../components/FlowFields.vue'
import AuthForm from '../components/AuthForm.vue'
import Modal from '../components/Modal.vue'
import Icon from '../components/Icon.vue'
import ErrorNotice from '../components/ErrorNotice.vue'
import LoadingState from '../components/LoadingState.vue'
const route = useRoute()
const auth = useAuth()
const id = String(route.params.id)
/** 本地暂存按问卷与发布版本隔离，换版本后不会把旧输入带到新题面。 */
interface FillDraft {
  events?: FlowEvent[]
}
/** 题面载入时的账号快照：草稿与未决提交都只使用这个作用域。 */
interface LoadedScope {
  accountId: string | null
  scope: string
  versionId: string
}
const meta = shallowRef<PublicSurveyMeta | null>(null)
const survey = shallowRef<PublicSurvey | null>(null)
const receipt = shallowRef<Receipt | null>(null)
const receiptPanel = ref<HTMLElement | null>(null)
const flowDraft = ref<FlowEvent[]>([])
const errors = ref<Record<string, string>>({})
const error = ref('')
const loading = ref(true)
const busy = ref(false)
const loginNeeded = ref(false)
const uncertain = ref(false)
const versionChanged = ref(false)
/** 当前登录账号与冻结提交账号不一致：保留原提交，只提示切回原账号。 */
const accountMismatch = ref(false)
const replayed = ref<{ applied: number; total: number } | null>(null)
const pending = shallowRef<FrozenPending | null>(null)
const loadedScope = shallowRef<LoadedScope | null>(null)
const flowFields = ref<InstanceType<typeof FlowFields>>()
const flowAnswered = ref(0)
/** 「登录限次」但当前没有会话：先展示上下文与登录引导，不直接撞 401。 */
const loginRequired = computed(() => meta.value?.mode === 'login_once' && !auth.user)
const draftKey = computed(() =>
  loadedScope.value ? draftKeyFor(id, loadedScope.value.versionId, loadedScope.value.scope) : '',
)
/** 当前要核对的账号：匿名始终 anonymous；登录限次缺账号时为 null，不降级为匿名。 */
function currentScope(): string | null {
  return scopeFor(survey.value?.mode, auth.user?.id ?? null)
}
function flowChanged(count: number) {
  flowAnswered.value = count
  errors.value = {}
  scheduleSave()
}
const answered = computed(() => flowAnswered.value)
/** 0/0 表示本次没有可提示的暂存；全量、部分与全失败三种结果都必须明确告知。 */
const replayNotice = computed(() => {
  const info = replayed.value
  if (!info || info.total <= 0) return ''
  if (info.applied >= info.total) return '已恢复上次未提交的全部填写进度，可继续填写或清空重试。'
  if (info.applied <= 0)
    return '上次未提交的填写进度与当前题面不兼容，未能恢复；当前为空白，请重新填写或清空暂存。'
  return `已恢复上次未提交填写进度的前 ${info.applied} 项（共 ${info.total} 项），其余内容与当前题面不兼容，未恢复。可继续填写或清空重试。`
})
function replayedDraft(applied: number, total: number) {
  if (!Number.isFinite(applied) || !Number.isFinite(total) || total <= 0) {
    replayed.value = null
    return
  }
  const totalCount = Math.floor(total)
  replayed.value = {
    applied: Math.min(Math.max(0, Math.floor(applied)), totalCount),
    total: totalCount,
  }
}
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
  const draft: FillDraft = { events: flowFields.value?.events() ?? [] }
  if (!draft.events?.length) {
    // 恢复失败时保留原暂存，等用户明确清空；不能让重放后的空事件把它悄悄删掉。
    const failedReplay = replayed.value && replayed.value.applied < replayed.value.total
    if (!failedReplay) clearDraft()
    return
  }
  try {
    localStorage.setItem(draftKey.value, JSON.stringify(draft))
  } catch {
    // 隐私模式等存储不可用时静默跳过，填写本身不受影响。
  }
}
function readPending(scope: string): FrozenPending | null {
  try {
    const raw = localStorage.getItem(pendingKeyFor(id, scope))
    if (!raw) return null
    // 作用域只来自存储键；本地载荷仍按不可信输入校验，形状完整才冻结。
    return freezePending(decode<unknown>(raw), scope)
  } catch {
    return null
  }
}
function writePending(frozen: FrozenPending) {
  try {
    localStorage.setItem(pendingKeyFor(id, frozen.scope), JSON.stringify(frozen.payload))
  } catch {
    // 存储不可用时只在本次会话内保留原载荷。
  }
}
function clearPending(scope: string) {
  try {
    localStorage.removeItem(pendingKeyFor(id, scope))
  } catch {}
}
let saveTimer: ReturnType<typeof setTimeout> | undefined
function scheduleSave() {
  clearTimeout(saveTimer)
  saveTimer = setTimeout(writeDraft, 400)
}
function removeDraft(key: string) {
  try {
    localStorage.removeItem(key)
  } catch {}
}
function clearDraft() {
  clearTimeout(saveTimer)
  saveTimer = undefined
  if (draftKey.value) removeDraft(draftKey.value)
}
/** 流程页「清空重填」：同时丢弃已回放的进度，避免下次初始化又被恢复。 */
function resetFlowDraft() {
  flowDraft.value = []
  replayed.value = null
  clearDraft()
}
function applyStoredDraft(result: PublicSurvey, scope: string) {
  flowDraft.value = []
  replayed.value = null
  const draft = readDraft(draftKeyFor(id, result.version_id, scope))
  if (!draft || !Array.isArray(draft.events) || !draft.events.length) return
  // 只交付事件；是否真正恢复由 FlowFields 的 replayed 事件决定，父端不提前宣告成功。
  flowDraft.value = draft.events
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
  // 题面加载时冻结账号：此后即使 auth.user 变化，草稿与未决提交也不改作用域。
  const accountId = auth.user?.id ?? null
  try {
    const result = await api<PublicSurvey>(`/api/public/surveys/${id}`)
    const scope = scopeFor(result.mode, accountId)
    if (!scope) {
      // 登录限次缺少账号时不能把题面落到 anonymous 作用域。
      survey.value = null
      loadedScope.value = null
      loginNeeded.value = true
      return
    }
    survey.value = result
    loadedScope.value = {
      accountId,
      scope,
      versionId: result.version_id,
    }
    versionChanged.value = false
    uncertain.value = false
    accountMismatch.value = false
    pending.value = null
    receipt.value = null
    errors.value = {}
    flowAnswered.value = 0
    applyStoredDraft(result, loadedScope.value.scope)
    // 未决提交只在版本一致时回放，保证“重试原提交”沿用原来的幂等键。
    const storedPending = readPending(loadedScope.value.scope)
    if (storedPending && storedPending.payload.version_id === result.version_id) {
      pending.value = storedPending
      uncertain.value = true
    } else if (storedPending) {
      clearPending(storedPending.scope)
    }
  } catch (reason) {
    if (reason instanceof ApiError && reason.status === 401) loginNeeded.value = true
    else error.value = message(reason)
  } finally {
    loading.value = false
  }
}
function resetFill() {
  if (busy.value || uncertain.value || versionChanged.value) return
  // 恢复提示存在时即使没有答案也允许明确清空损坏或不可恢复的暂存。
  if (answered.value && !window.confirm('确定清空当前已填写的回答吗？')) return
  error.value = ''
  errors.value = {}
  flowAnswered.value = 0
  replayed.value = null
  if (flowFields.value) flowFields.value.reset()
  else flowDraft.value = []
  clearDraft()
}
/** 放弃冻结的未决提交并解锁表单；只在用户明确接受重复提交风险后执行。 */
function abandonPending() {
  if (
    !window.confirm(
      '如果上次提交其实已经成功，放弃后重新填写可能再提交一次（登录限次问卷仍只计入首次）。确定放弃吗？',
    )
  )
    return
  const scope = pending.value?.scope
  pending.value = null
  uncertain.value = false
  accountMismatch.value = false
  error.value = ''
  if (scope) clearPending(scope)
}
async function loginSuccess() {
  loginNeeded.value = false
  error.value = ''
  // 冻结作用域是唯一依据：B 登录后既不发送 A 的载荷，也不把幂等键迁到 B。
  if (
    pending.value &&
    pending.value.scope !== ANONYMOUS_SCOPE &&
    pending.value.scope !== auth.user?.id
  ) {
    accountMismatch.value = true
    loginNeeded.value = true
    error.value = '当前登录账号与原提交账号不一致，请切回原账号后重试；原回答与提交标识仍然保留。'
    return
  }
  accountMismatch.value = false
  if (!survey.value) {
    await loadSurvey()
    return
  }
  // 登录限次换成另一个账号后重新载入题面，避免继续显示上一账号的草稿。
  if (survey.value.mode === 'login_once' && loadedScope.value?.accountId !== auth.user?.id) {
    writeDraft()
    await loadSurvey()
  }
}
async function focusError() {
  const first = survey.value?.document.questions.find((question) => errors.value[question.id])
  if (first) await flowFields.value?.focusQuestion(first.id)
}
async function submit(flowAnswers?: Answer[]) {
  if (!survey.value || busy.value || receipt.value || versionChanged.value) return
  error.value = ''
  // 登录限次必须在首次冻结载荷前已有账号；未登录输入继续留在草稿，不冻结为匿名作用域。
  if (survey.value.mode === 'login_once' && !auth.user) {
    loginNeeded.value = true
    return
  }
  const loaded = loadedScope.value
  const scope = currentScope()
  // 首次冻结只能使用题面载入时的作用域，并且当前账号必须仍是载入账号。
  if (!loaded || !scope || loaded.scope !== scope) {
    accountMismatch.value = true
    loginNeeded.value = true
    error.value = '当前登录账号与原提交账号不一致，请切回原账号后重试；原回答与提交标识仍然保留。'
    return
  }
  if (!pending.value) {
    flowAnswers ??= flowFields.value?.submissionAnswers()
    if (!flowAnswers) return
    errors.value = {}
    pending.value = {
      scope: loaded.scope,
      payload: {
        version_id: survey.value.version_id,
        // UUIDv7 同样满足后端对幂等秘密的版本校验。
        idempotency_key: newId(),
        answers: flowAnswers.map((answer) => ({
          ...answer,
          value: Array.isArray(answer.value) ? [...answer.value] : answer.value,
        })),
      },
    }
  }
  const frozen = pending.value
  if (!frozen) return
  // 发送前再次核对冻结作用域：B 登录后不能拿 A 的答案与幂等键去 POST。
  if (frozen.scope !== loaded.scope || frozen.scope !== scope) {
    accountMismatch.value = true
    loginNeeded.value = true
    error.value = '当前登录账号与原提交账号不一致，请切回原账号后重试；原回答与提交标识仍然保留。'
    return
  }
  accountMismatch.value = false
  // 请求发出前先冻结载荷：请求途中关闭页面也能用原幂等键重试。
  writePending(frozen)
  busy.value = true
  try {
    // 匿名不声明账号；登录限次把冻结账号交给后端核对真实 Cookie 身份。
    const expectedAccount = frozen.scope === ANONYMOUS_SCOPE ? undefined : frozen.scope
    const result = await api<Receipt>(
      `/api/public/surveys/${id}/responses`,
      'POST',
      frozen.payload,
      { expectedAccount },
    )
    if (result.status !== 'submitted')
      throw new ApiError(0, 'invalid_response', '尚未收到完整回执，请重试原提交。')
    receipt.value = result
    pending.value = null
    uncertain.value = false
    accountMismatch.value = false
    replayed.value = null
    // 成功清理使用冻结作用域，不随此后 auth.user 或题面状态改变。
    clearDraft()
    removeDraft(draftKeyFor(id, frozen.payload.version_id, frozen.scope))
    clearPending(frozen.scope)
    window.scrollTo({ top: 0 })
    await nextTick()
    receiptPanel.value?.focus()
  } catch (reason) {
    error.value = message(reason)
    // 同一幂等键连同原载荷已经落库：这是明确结果，不再按“结果未知”锁定表单。
    const conflict = reason instanceof ApiError && reason.code === 'idempotency_conflict'
    const accountChanged = reason instanceof ApiError && reason.code === 'account_changed'
    if (conflict)
      error.value =
        '这次提交的标识此前已记录过另一份回答，原提交不会重复计入。请核对答案后重新提交。'
    if (accountChanged)
      error.value =
        '当前浏览器登录的账号与原提交账号不一致。请切换回原账号后重试；原回答与提交标识仍然保留。'
    if (reason instanceof ApiError) {
      if (reason.question_id) {
        errors.value = { [reason.question_id]: reason.message }
        await focusError()
      }
      if (reason.status === 401) loginNeeded.value = true
      if (reason.code === 'version_changed') versionChanged.value = true
    }
    // account_changed 是拒绝写入但必须保留原提交；其它拿不到明确结果的情况同样锁定。
    uncertain.value =
      accountChanged ||
      (!conflict &&
        (reason instanceof ApiError
          ? reason.status === 0 || reason.status >= 500 || reason.status === 401
          : true))
    if (accountChanged) {
      accountMismatch.value = true
      loginNeeded.value = true
    }
    // 结果未知时保留请求前已写入的冻结载荷；明确失败才按冻结作用域丢弃。
    if (!uncertain.value) {
      pending.value = null
      clearPending(frozen.scope)
    }
  } finally {
    busy.value = false
    await nextTick()
    if (!uncertain.value && !versionChanged.value) await focusError()
  }
}
function leave(event: BeforeUnloadEvent) {
  writeDraft()
  if (!receipt.value && (busy.value || answered.value > 0 || uncertain.value)) {
    event.preventDefault()
    event.returnValue = ''
  }
}
onBeforeRouteLeave(() => {
  writeDraft()
  return (
    !!receipt.value ||
    (!busy.value && !answered.value && !uncertain.value) ||
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
      <!-- 保持挂载以保留 A 的草稿与冻结状态，但账号不一致时不能透过弹窗背景看到原答案。 -->
      <form v-else-if="survey" v-show="!accountMismatch" novalidate @submit.prevent="submit()">
        <div v-if="replayNotice" class="notice mt">
          <div class="grow">{{ replayNotice }}</div>
          <button
            type="button"
            class="text-button"
            :disabled="busy || uncertain || versionChanged"
            @click="resetFill"
          >
            清空重填
          </button>
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
          <div class="grow">
            上一次提交没有得到确认（网络或服务中断），回答已暂时锁定，避免重复提交。点「重试原提交」获取结果，或放弃这次提交重新填写。
          </div>
          <button
            type="button"
            class="text-button danger-text"
            :disabled="busy"
            @click="abandonPending"
          >
            放弃并重新填写
          </button>
        </div>
        <FlowFields
          v-if="survey.document.questions.length"
          ref="flowFields"
          :document="survey.document"
          :restore="flowDraft"
          :errors="errors"
          :disabled="busy || uncertain || versionChanged || accountMismatch"
          :submit-disabled="loginNeeded"
          :hide-footer="uncertain || versionChanged || accountMismatch"
          @change="flowChanged"
          @reset="resetFlowDraft"
          @replayed="replayedDraft"
          @submit="submit"
        />
        <p v-else class="notice mt">本问卷没有需要回答的题目。</p>
        <div v-if="uncertain || versionChanged" class="fill-footer actions between">
          <div class="actions fill-footer-left">
            <a
              v-if="versionChanged"
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
              {{ busy ? '正在提交…' : '重试原提交' }}
            </button>
          </div>
        </div>
      </form>
      <LoadingState v-else-if="!error" />
    </template>
  </div>
  <Modal v-if="loginNeeded && meta" title="请重新登录后继续" :dismissible="false">
    <p v-if="accountMismatch" class="subtle mb">
      当前登录的不是原提交账号。原回答与提交标识仍然保留，请使用原账号登录后原样重试；系统不会把这份提交转给当前账号。
    </p>
    <p v-else class="subtle mb">当前回答仍然保留。请登录后重试提交。</p>
    <ErrorNotice :message="error" />
    <AuthForm
      :initial-username="accountMismatch ? '' : auth.user?.username"
      @success="loginSuccess"
    />
    <div class="actions between mt">
      <RouterLink class="text-button" to="/">先返回首页</RouterLink>
    </div>
  </Modal>
</template>
