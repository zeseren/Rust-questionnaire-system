<!-- 管理员只访问治理接口；不通过此页面读取他人的答卷。 -->
<script setup lang="ts">
import { computed, ref, shallowRef, watch } from 'vue'
import { api, message, query } from '../api'
import { dateTime, statusNames } from '../format'
import { useResource } from '../resource'
import { useAuth, useNotice } from '../stores'
import type { AdminSurvey, AdminUser, Audit, SurveyDocument } from '../types'
import ErrorNotice from '../components/ErrorNotice.vue'
import LoadingState from '../components/LoadingState.vue'
import Pagination from '../components/Pagination.vue'
import Modal from '../components/Modal.vue'
const auth = useAuth()
const notice = useNotice()
const tab = ref<'users' | 'surveys' | 'audits'>('users')
const labels = { users: '用户管理', surveys: '问卷治理', audits: '操作审计' }
const page = ref(1)
const list = useResource(
  () =>
    api<{ total: number; items: (AdminUser | AdminSurvey | Audit)[] }>(
      `/api/admin/${tab.value}${query({ page: page.value, page_size: 20 })}`,
    ),
  [tab, page],
)
const users = computed(() => list.data.value?.items as AdminUser[] | undefined)
const surveys = computed(() => list.data.value?.items as AdminSurvey[] | undefined)
const audits = computed(() => list.data.value?.items as Audit[] | undefined)
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
const document = shallowRef<{
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
  document.value = null
  inspectError.value = ''
  try {
    document.value = await api(`/api/admin/surveys/${id}`)
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
        :aria-selected="tab === key"
        @click="tab = key"
      >
        {{ label }}
      </button>
    </div>
  </div>
  <ErrorNotice :message="list.error.value" retry @retry="list.refresh" />
  <LoadingState v-if="list.loading.value" />
  <template v-if="list.data.value">
    <div v-if="list.data.value.items.length" class="table-container">
      <table v-if="tab === 'users'">
        <thead>
          <tr>
            <th>用户名</th>
            <th>角色</th>
            <th>状态</th>
            <th>创建时间</th>
            <th>操作</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="user in users" :key="user.id">
            <td>{{ user.username }}</td>
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
            <th>回收状态</th>
            <th>治理状态</th>
            <th>操作</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="survey in surveys" :key="survey.id">
            <td>{{ survey.title || '无标题问卷' }}</td>
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
      <div class="actions end">
        <button type="button" class="button" :disabled="busy" @click="selected = null">取消</button>
        <button class="button button-primary" :disabled="busy || !reason.trim()">
          {{ busy ? '正在处理…' : `确认${operation}` }}
        </button>
      </div>
    </form>
  </Modal>
  <Modal v-if="inspectingId" title="问卷治理详情" wide @close="inspectingId = ''">
    <LoadingState v-if="inspecting" />
    <ErrorNotice :message="inspectError" />
    <template v-if="document">
      <h2>{{ document.survey.title || '无标题问卷' }}</h2>
      <p class="hint mt">
        {{ statusNames[document.survey.status] }} ·
        {{ document.survey.taken_down ? '已下架' : '未下架' }}
      </p>
      <template v-if="document.published_document">
        <p class="plain-text subtle mt">{{ document.published_document.description }}</p>
        <p class="plain-text hint mt">收集目的：{{ document.published_document.purpose }}</p>
        <div
          v-for="(question, index) in document.published_document.questions"
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
  </Modal>
</template>
