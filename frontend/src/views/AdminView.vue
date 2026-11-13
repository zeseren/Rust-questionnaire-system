<!-- [入口在哪] 平台管理页的用户/问卷/审计表格；每行操作列就近提供启停、下架、查看题面。 -->
<!-- [用户在第 1 步后悔了怎么删/退] 治理确认弹窗左侧「取消」可随时退出；长题面弹窗底部固定「关闭」。 -->
<!-- [主提交通道在哪] 确认弹窗右侧唯一主按钮：破坏性操作使用朱红 button-danger，正向恢复才用 button-primary。 -->
<!-- 管理员只访问治理接口；不通过此页面读取他人的答卷。 -->
<script setup lang="ts">
import { computed, ref, shallowRef, watch } from 'vue'
import { api, message, query } from '../api'
import { dateTime, statusNames } from '../format'
import { useResource } from '../resource'
import { useAuth, useNotice } from '../stores'
import type { AdminSurvey, AdminUser, Audit, Status, SurveyDocument } from '../types'
import ErrorNotice from '../components/ErrorNotice.vue'
import LoadingState from '../components/LoadingState.vue'
import Pagination from '../components/Pagination.vue'
import Modal from '../components/Modal.vue'
const auth = useAuth()
const notice = useNotice()
const tab = ref<'users' | 'surveys' | 'audits'>('users')
const labels = { users: '用户管理', surveys: '问卷治理', audits: '操作审计' }
const statuses: Status[] = ['unpublished', 'collecting', 'closed', 'archived']
const page = ref(1)
// 筛选输入与已应用的取值分离，只有点击「应用筛选」才重新请求。
const surveyTitle = ref('')
const surveyStatus = ref<Status | ''>('')
const surveyTakenDown = ref<'' | 'true' | 'false'>('')
const auditAction = ref('')
const auditActor = ref('')
const auditTarget = ref('')
/** 账号 ID 与目标 ID 会直接拼进查询串，格式错误只会在后端换来一次必然 400。 */
const UUID_TEXT = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i
const filterError = ref('')
function invalidUuid(value: string) {
  const trimmed = value.trim()
  return trimmed !== '' && !UUID_TEXT.test(trimmed)
}
const applied = ref({
  title: '',
  status: '' as Status | '',
  taken_down: '' as '' | 'true' | 'false',
  action: '',
  actor_id: '',
  target_id: '',
})
function applyFilters() {
  if (invalidUuid(auditActor.value) || invalidUuid(auditTarget.value)) {
    filterError.value = '账号 ID 与目标 ID 需要是 36 位连字符 UUID；留空表示不筛选。'
    return
  }
  filterError.value = ''
  page.value = 1
  applied.value = {
    title: surveyTitle.value.trim(),
    status: surveyStatus.value,
    taken_down: surveyTakenDown.value,
    action: auditAction.value,
    actor_id: auditActor.value.trim(),
    target_id: auditTarget.value.trim(),
  }
}
function resetFilters() {
  surveyTitle.value = ''
  surveyStatus.value = ''
  surveyTakenDown.value = ''
  auditAction.value = ''
  auditActor.value = ''
  auditTarget.value = ''
  applyFilters()
}
type AdminListResult =
  | { kind: 'users'; total: number; items: AdminUser[] }
  | { kind: 'surveys'; total: number; items: AdminSurvey[] }
  | { kind: 'audits'; total: number; items: Audit[] }
const list = useResource<AdminListResult>(async () => {
  const suffix = query({
    page: page.value,
    page_size: 20,
    ...(tab.value === 'surveys'
      ? {
          title: applied.value.title,
          status: applied.value.status,
          taken_down: applied.value.taken_down,
        }
      : tab.value === 'audits'
        ? {
            action: applied.value.action,
            actor_id: applied.value.actor_id,
            target_id: applied.value.target_id,
          }
        : {}),
  })
  if (tab.value === 'users') {
    const result = await api<{ total: number; items: AdminUser[] }>(`/api/admin/users${suffix}`)
    return { kind: 'users' as const, total: result.total, items: result.items }
  }
  if (tab.value === 'surveys') {
    const result = await api<{ total: number; items: AdminSurvey[] }>(`/api/admin/surveys${suffix}`)
    return { kind: 'surveys' as const, total: result.total, items: result.items }
  }
  const result = await api<{ total: number; items: Audit[] }>(`/api/admin/audits${suffix}`)
  return { kind: 'audits' as const, total: result.total, items: result.items }
}, [tab, page, applied])
const users = computed(() => {
  const data = list.data.value
  return data?.kind === 'users' ? data.items : undefined
})
const surveys = computed(() => {
  const data = list.data.value
  return data?.kind === 'surveys' ? data.items : undefined
})
const audits = computed(() => {
  const data = list.data.value
  return data?.kind === 'audits' ? data.items : undefined
})
const selected = shallowRef<
  { kind: 'users'; item: AdminUser } | { kind: 'surveys'; item: AdminSurvey } | null
>(null)
const reason = ref('')
const busy = ref(false)
const error = ref('')
const operation = computed(() =>
  selected.value?.kind === 'users'
    ? selected.value.item.disabled
      ? '启用账号'
      : '禁用账号'
    : selected.value?.item.taken_down
      ? '恢复问卷'
      : '下架问卷',
)
const destructive = computed(() => {
  const target = selected.value
  if (!target) return false
  return target.kind === 'users' ? !target.item.disabled : !target.item.taken_down
})
const detail = shallowRef<{
  survey: AdminSurvey
  published_document: SurveyDocument | null
} | null>(null)
const inspecting = ref(false)
const inspectingId = ref('')
const inspectError = ref('')
watch(tab, () => {
  page.value = 1
  error.value = ''
})
function choose(value: NonNullable<typeof selected.value>) {
  selected.value = value
  reason.value = ''
  error.value = ''
}
async function govern() {
  if (!selected.value || !reason.value.trim() || busy.value) return
  busy.value = true
  error.value = ''
  try {
    const target = selected.value
    const body =
      target.kind === 'users'
        ? { disabled: !target.item.disabled, reason: reason.value.trim() }
        : { taken_down: !target.item.taken_down, reason: reason.value.trim() }
    await api(`/api/admin/${target.kind}/${target.item.id}`, 'PATCH', body)
    notice.show(`${operation.value}成功，操作已记录。`)
    selected.value = null
    await list.refresh()
  } catch (failure) {
    error.value = message(failure)
  } finally {
    busy.value = false
  }
}
async function inspect(id: string) {
  inspecting.value = true
  inspectingId.value = id
  detail.value = null
  inspectError.value = ''
  try {
    detail.value = await api(`/api/admin/surveys/${id}`)
  } catch (failure) {
    inspectError.value = message(failure)
  } finally {
    inspecting.value = false
  }
}
const actionNames: Record<string, string> = {
  disable_user: '禁用账号',
  enable_user: '启用账号',
  take_down: '下架问卷',
  restore_survey: '恢复问卷',
}
</script>
<template>
  <div class="page-heading">
    <div>
      <p class="eyebrow">PLATFORM GOVERNANCE &amp; AUDIT</p>
      <h1>平台管理</h1>
      <p class="heading-note">每一次治理操作，都有据可查。</p>
    </div>
    <button class="button" :disabled="list.loading.value" @click="list.refresh">刷新</button>
  </div>
  <div class="toolbar">
    <div class="tabs" role="group" aria-label="管理内容">
      <button
        v-for="(label, key) in labels"
        :key="key"
        class="tab"
        :aria-pressed="tab === key"
        @click="tab = key"
      >
        {{ label }}
      </button>
    </div>
  </div>
  <form v-if="tab !== 'users'" class="panel admin-filter" @submit.prevent="applyFilters">
    <template v-if="tab === 'surveys'">
      <label class="field">
        标题关键词
        <input v-model="surveyTitle" type="search" placeholder="按标题筛选" autocomplete="off" />
      </label>
      <label class="field">
        回收状态
        <select v-model="surveyStatus">
          <option value="">全部状态</option>
          <option v-for="status in statuses" :key="status" :value="status">
            {{ statusNames[status] }}
          </option>
        </select>
      </label>
      <label class="field">
        下架状态
        <select v-model="surveyTakenDown">
          <option value="">全部</option>
          <option value="true">已下架</option>
          <option value="false">未下架</option>
        </select>
      </label>
    </template>
    <template v-else>
      <label class="field">
        操作类型
        <select v-model="auditAction">
          <option value="">全部操作</option>
          <option v-for="(label, action) in actionNames" :key="action" :value="action">
            {{ label }}
          </option>
        </select>
      </label>
      <label class="field">
        操作者账号 ID
        <input
          v-model="auditActor"
          placeholder="36 位连字符 UUID，留空表示全部"
          autocomplete="off"
          :aria-invalid="invalidUuid(auditActor)"
        />
      </label>
      <label class="field">
        目标 ID
        <input
          v-model="auditTarget"
          placeholder="36 位连字符 UUID，留空表示全部"
          autocomplete="off"
          :aria-invalid="invalidUuid(auditTarget)"
        />
      </label>
    </template>
    <p v-if="filterError" class="accent hint" style="flex-basis: 100%">{{ filterError }}</p>
    <button type="button" class="text-button danger-text" @click="resetFilters">重置筛选</button>
    <button class="button button-primary">应用筛选</button>
  </form>
  <ErrorNotice :message="list.error.value" retry @retry="list.refresh" />
  <LoadingState v-if="list.loading.value" />
  <template v-if="list.data.value">
    <div v-if="list.data.value.items.length" class="table-container">
      <table v-if="tab === 'users'">
        <thead>
          <tr>
            <th>用户 / 账号 ID</th>
            <th>角色</th>
            <th>状态</th>
            <th>创建时间</th>
            <th>操作</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="user in users" :key="user.id">
            <td>
              <div>{{ user.username }}</div>
              <code class="hint">{{ user.id }}</code>
            </td>
            <td>{{ user.role === 'admin' ? '管理员' : '普通用户' }}</td>
            <td>{{ user.disabled ? '已禁用' : '正常' }}</td>
            <td>{{ dateTime(user.created_at) }}</td>
            <td>
              <button
                class="text-button"
                :class="{ 'danger-text': !user.disabled }"
                :disabled="user.id === auth.user?.id"
                @click="choose({ kind: 'users', item: user })"
              >
                {{ user.disabled ? '启用' : '禁用' }}
              </button>
            </td>
          </tr>
        </tbody>
      </table>
      <table v-else-if="tab === 'surveys'">
        <thead>
          <tr>
            <th>问卷</th>
            <th>发布者账号 ID</th>
            <th>回收状态</th>
            <th>治理状态</th>
            <th>操作</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="survey in surveys" :key="survey.id">
            <td>{{ survey.title || '无标题问卷' }}</td>
            <td>
              <code>{{ survey.owner_id }}</code>
            </td>
            <td>{{ statusNames[survey.status] }}</td>
            <td>{{ survey.taken_down ? '已下架' : '正常' }}</td>
            <td>
              <div class="actions">
                <button
                  class="text-button accent"
                  :disabled="inspecting"
                  @click="inspect(survey.id)"
                >
                  查看题面
                </button>
                <span class="grow" aria-hidden="true"></span>
                <button
                  class="text-button"
                  :class="{ 'danger-text': !survey.taken_down }"
                  @click="choose({ kind: 'surveys', item: survey })"
                >
                  {{ survey.taken_down ? '恢复' : '下架' }}
                </button>
              </div>
            </td>
          </tr>
        </tbody>
      </table>
      <table v-else>
        <thead>
          <tr>
            <th>操作时间</th>
            <th>操作</th>
            <th>原因</th>
            <th>操作者 / 目标</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="audit in audits" :key="audit.id">
            <td>{{ dateTime(audit.created_at) }}</td>
            <td>{{ actionNames[audit.action] || audit.action }}</td>
            <td class="plain-text">{{ audit.reason }}</td>
            <td>
              <code>
                {{ audit.actor_id }}
                <br />
                {{ audit.target_id }}
              </code>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
    <div v-else class="empty-state">
      <h3>暂无记录</h3>
      <p>当前列表没有可展示的内容。</p>
    </div>
    <Pagination :page="page" :total="list.data.value.total" @change="page = $event" />
  </template>
  <Modal v-if="selected" :title="operation" :dismissible="!busy" @close="selected = null">
    <form class="form-stack" @submit.prevent="govern">
      <p class="subtle">
        {{ selected.kind === 'users' ? selected.item.username : selected.item.title }}
        <br />
        {{
          selected.kind === 'surveys' && selected.item.taken_down
            ? '恢复只解除下架，不会自动恢复已关闭或已归档问卷的回收。'
            : selected.kind === 'users' && !selected.item.disabled
              ? '禁用后现有会话失效，该用户发布的问卷暂停接收新提交。'
              : '请填写具体原因，操作将记录在审计中。'
        }}
      </p>
      <ErrorNotice :message="error" />
      <label class="field">
        操作原因
        <textarea
          v-model="reason"
          required
          autofocus
          :disabled="busy"
          placeholder="说明本次操作的原因"
        />
      </label>
      <div class="actions between">
        <button type="button" class="button" :disabled="busy" @click="selected = null">取消</button>
        <button
          :class="destructive ? 'button button-danger' : 'button button-primary'"
          :disabled="busy || !reason.trim()"
        >
          {{ busy ? '正在处理…' : `确认${operation}` }}
        </button>
      </div>
    </form>
  </Modal>
  <Modal v-if="inspectingId" title="问卷治理详情" wide @close="inspectingId = ''">
    <LoadingState v-if="inspecting" />
    <ErrorNotice :message="inspectError" />
    <template v-if="detail">
      <h2>{{ detail.survey.title || '无标题问卷' }}</h2>
      <p class="hint mt">
        发布者账号 ID：
        <code>{{ detail.survey.owner_id }}</code>
      </p>
      <p class="hint mt">
        {{ statusNames[detail.survey.status] }} ·
        {{ detail.survey.taken_down ? '已下架' : '未下架' }}
      </p>
      <template v-if="detail.published_document">
        <p class="plain-text subtle mt">{{ detail.published_document.description }}</p>
        <p class="plain-text hint mt">收集目的：{{ detail.published_document.purpose }}</p>
        <div
          v-for="(question, index) in detail.published_document.questions"
          :key="question.id"
          class="text-answer"
        >
          <h3>{{ index + 1 }}. {{ question.title }}</h3>
          <p v-if="'options' in question" class="plain-text">
            {{ question.options.map((option) => option.label).join('\n') }}
          </p>
        </div>
      </template>
      <p v-else class="hint mt">尚无发布版本，暂无公开题面。</p>
    </template>
    <template #footer>
      <div class="actions between">
        <span class="hint">只读查看，不会改变问卷状态。</span>
        <button type="button" class="button" @click="inspectingId = ''">关闭</button>
      </div>
    </template>
  </Modal>
</template>
