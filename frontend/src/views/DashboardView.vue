<!-- [入口在哪] 登录后首页与归档页的问卷卡片行，卡片底部操作列直接管理该问卷生命周期。 -->
<!-- [用户在第 1 步后悔了怎么删/退] 草稿卡片行内常驻「删除草稿」幽灵按钮（悬停变红 + 二次确认）；已发布卡片行内可直接关闭/恢复回收或归档，无需进入编辑器第 4 步。 -->
<!-- [主提交通道在哪] 页头「新建问卷」是唯一主按钮；空态按钮降级为次级，创建弹窗内右侧单独承载「创建草稿」。 -->
<!-- 列表和状态总数来自 API，不用示例答卷数填充界面。 -->
<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { RouterLink, useRoute, useRouter } from 'vue-router'
import { api, message, query } from '../api'
import { useResource } from '../resource'
import { dateTime, shareLink, statusNames } from '../format'
import { useNotice } from '../stores'
import type { Page, Status, SurveyDetail, SurveySummary } from '../types'
import Icon from '../components/Icon.vue'
import Modal from '../components/Modal.vue'
import ErrorNotice from '../components/ErrorNotice.vue'
import LoadingState from '../components/LoadingState.vue'
import Pagination from '../components/Pagination.vue'
import SurveyTemplatePicker from '../components/SurveyTemplatePicker.vue'
import { createSurveyTemplate, surveyTemplates } from '../survey-templates'
import type { SurveyTemplateId } from '../survey-templates'
import '../editor.css'
const route = useRoute()
const router = useRouter()
const notice = useNotice()
const archived = computed(() => route.name === 'archive')
const page = ref(1)
const filter = ref<Status | ''>(archived.value ? 'archived' : '')
const keyword = ref('')
const title = ref('')
const view = ref<'grid' | 'list'>('list')
const creating = ref(false)
const newTitle = ref('')
const selectedTemplate = ref<SurveyTemplateId | null>(null)
watch(selectedTemplate, (current, previous) => {
  const previousTitle = surveyTemplates.find((template) => template.id === previous)?.title ?? ''
  if (!newTitle.value || newTitle.value === previousTitle)
    newTitle.value = surveyTemplates.find((template) => template.id === current)?.title ?? ''
})
const busy = ref(false)
const actionError = ref('')
const statuses: Status[] = ['unpublished', 'collecting', 'closed', 'archived']
const list = useResource(
  () =>
    api<Page<SurveySummary>>(
      `/api/surveys${query({ title: keyword.value, status: filter.value, page: page.value, page_size: 12 })}`,
    ),
  [page, filter, keyword],
)
const counts = useResource(async () => {
  const totals: Record<Status, number> = {
    unpublished: 0,
    collecting: 0,
    closed: 0,
    archived: 0,
  }
  await Promise.all(
    statuses.map(async (status) => {
      const result = await api<Page<SurveySummary>>(
        `/api/surveys${query({ status, page_size: 1 })}`,
      )
      totals[status] = result.total
    }),
  )
  return totals
})
const total = computed(() =>
  counts.data.value
    ? Object.values(counts.data.value).reduce((sum, value) => sum + value, 0)
    : null,
)
watch([filter, keyword], () => {
  page.value = 1
})
function search() {
  keyword.value = title.value.trim()
  page.value = 1
}
function refresh() {
  list.refresh()
  counts.refresh()
}
function openCreate() {
  actionError.value = ''
  newTitle.value = ''
  selectedTemplate.value = null
  creating.value = true
}
async function create() {
  if (busy.value) return
  busy.value = true
  actionError.value = ''
  try {
    const survey = await api<SurveyDetail>('/api/surveys', 'POST', {
      document: { ...createSurveyTemplate(selectedTemplate.value), title: newTitle.value.trim() },
    })
    creating.value = false
    notice.show('问卷草稿已创建。')
    await router.push(`/surveys/${survey.id}/edit`)
  } catch (error) {
    actionError.value = message(error)
  } finally {
    busy.value = false
  }
}
async function duplicate(id: string) {
  if (busy.value) return
  busy.value = true
  actionError.value = ''
  try {
    const copied = await api<SurveyDetail>(`/api/surveys/${id}/copy`, 'POST')
    notice.show('已复制为新的草稿。')
    await router.push(`/surveys/${copied.id}/edit`)
  } catch (error) {
    actionError.value = message(error)
  } finally {
    busy.value = false
  }
}
type CardAction = 'delete' | 'close' | 'resume' | 'archive'
const pendingAction = ref<{ survey: SurveySummary; action: CardAction } | null>(null)
const actionNames: Record<CardAction, string> = {
  delete: '删除草稿',
  close: '关闭回收',
  resume: '恢复回收',
  archive: '归档问卷',
}
const actionDescriptions: Record<CardAction, string> = {
  delete: '这份未发布草稿将被永久删除，无法恢复。',
  close: '关闭后停止接收新答卷，已有答卷和统计仍然保留。',
  resume: '恢复后重新接受当前版本的答卷。请确认截止时间尚未到达。',
  archive: '归档不可撤销。归档后不能编辑或再次发布，历史答卷和统计仍可查看。',
}
// Rule 1：卡片行只在该状态确有可执行操作时渲染操作列，避免归档卡片出现空行。
function hasCardActions(survey: SurveySummary) {
  return survey.status !== 'archived'
}
function confirmAction(survey: SurveySummary, act: CardAction) {
  if (busy.value) return
  actionError.value = ''
  pendingAction.value = { survey, action: act }
}
function closeAction() {
  pendingAction.value = null
  actionError.value = ''
}
async function performAction() {
  if (!pendingAction.value || busy.value) return
  const { survey, action: act } = pendingAction.value
  busy.value = true
  actionError.value = ''
  try {
    if (act === 'delete') await api(`/api/surveys/${survey.id}`, 'DELETE')
    else await api<SurveyDetail>(`/api/surveys/${survey.id}/${act}`, 'POST')
    notice.show(act === 'delete' ? '草稿已删除。' : `${actionNames[act]}成功。`)
    pendingAction.value = null
    refresh()
  } catch (error) {
    actionError.value = message(error)
  } finally {
    busy.value = false
  }
}
async function copySurveyLink(survey: SurveySummary) {
  try {
    await navigator.clipboard.writeText(shareLink(survey.id))
    notice.show('填写链接已复制。')
  } catch {
    notice.show('请选中链接后手动复制。')
  }
}
</script>
<template>
  <div class="page-heading">
    <div>
      <p class="eyebrow">
        {{ archived ? 'ARCHIVE · 历史案卷归档' : 'SURVEY ARCHIVE & TELEMETRY' }}
      </p>
      <h1>{{ route.meta.title }}</h1>
      <p class="heading-note">
        {{ archived ? '查看已归档问卷与历史答卷。' : '管理问卷、查看回答，了解最新进展。' }}
      </p>
    </div>
    <button class="button button-primary" @click="openCreate">
      <Icon name="plus" />
      新建问卷
    </button>
  </div>
  <ErrorNotice :message="counts.error.value" retry @retry="counts.refresh" />
  <section v-if="route.name === 'home'" class="overview" aria-label="问卷概览">
    <div class="metric">
      <div class="metric-label">
        <Icon name="forms" />
        全部问卷
      </div>
      <div class="metric-data">
        <span class="metric-value">{{ total ?? '—' }}</span>
        <span class="metric-unit">份</span>
      </div>
    </div>
    <div
      v-for="status in ['collecting', 'unpublished', 'archived'] as const"
      :key="status"
      class="metric"
    >
      <div class="metric-label">
        <Icon
          :name="
            status === 'collecting' ? 'collect' : status === 'archived' ? 'archive' : 'document'
          "
        />
        {{ statusNames[status] }}
      </div>
      <div class="metric-data">
        <span class="metric-value">{{ counts.data.value?.[status] ?? '—' }}</span>
        <span class="metric-unit">份</span>
      </div>
    </div>
  </section>
  <section class="surveys-section">
    <div class="section-heading">
      <div class="section-title">
        <h2>{{ archived ? '归档问卷' : '我的问卷' }}</h2>
        <span v-if="list.data.value" class="section-count">{{ list.data.value.total }}</span>
      </div>
      <button class="text-button" :disabled="list.loading.value" @click="refresh">刷新列表</button>
    </div>
    <div class="toolbar">
      <div class="tabs" role="group" aria-label="按状态筛选">
        <button v-if="!archived" class="tab" :aria-pressed="filter === ''" @click="filter = ''">
          全部
        </button>
        <template v-for="status in statuses" :key="status">
          <button
            v-if="!archived || status === 'archived'"
            class="tab"
            :aria-pressed="filter === status"
            @click="filter = status"
          >
            {{ statusNames[status] }}
            <span class="tab-count">{{ counts.data.value?.[status] ?? '' }}</span>
          </button>
        </template>
      </div>
      <div class="toolbar-right">
        <form class="search-field" role="search" @submit.prevent="search">
          <Icon name="search" />
          <label class="sr-only" for="survey-search">搜索问卷标题</label>
          <input
            id="survey-search"
            v-model="title"
            type="search"
            placeholder="搜索问卷…"
            @search="search"
          />
          <button type="submit" class="search-submit" aria-label="搜索">
            <Icon name="arrow" />
          </button>
        </form>
        <div class="view-control" role="group" aria-label="问卷显示方式">
          <button
            class="icon-button"
            aria-label="卡片视图"
            :aria-pressed="view === 'grid'"
            @click="view = 'grid'"
          >
            <Icon name="grid" />
          </button>
          <button
            class="icon-button"
            aria-label="列表视图"
            :aria-pressed="view === 'list'"
            @click="view = 'list'"
          >
            <Icon name="list" />
          </button>
        </div>
      </div>
    </div>
    <ErrorNotice v-if="!creating" :message="actionError" />
    <ErrorNotice :message="list.error.value" retry @retry="list.refresh" />
    <LoadingState v-if="list.loading.value" />
    <template v-if="list.data.value">
      <div
        v-if="list.data.value.items.length"
        class="survey-grid"
        :class="{ 'is-list': view === 'list' }"
      >
        <article
          v-for="(survey, index) in list.data.value.items"
          :key="survey.id"
          class="survey-card"
        >
          <div class="card-top">
            <span class="survey-icon" aria-hidden="true">
              {{ String((page - 1) * 12 + index + 1).padStart(2, '0') }}
            </span>
            <span
              class="status"
              :class="{
                'status-draft': survey.status === 'unpublished',
                'status-closed': ['closed', 'archived'].includes(survey.status),
              }"
            >
              {{ statusNames[survey.status] }}
            </span>
            <button
              class="icon-button card-more"
              :aria-label="`复制问卷：${survey.title || '无标题问卷'}`"
              title="复制问卷"
              :disabled="busy"
              @click="duplicate(survey.id)"
            >
              <Icon name="copy" />
            </button>
          </div>
          <div class="card-body">
            <h3 class="card-title">
              <RouterLink
                :to="`/surveys/${survey.id}/${survey.status === 'archived' ? 'reports' : 'edit'}`"
              >
                {{ survey.title || '无标题问卷' }}
              </RouterLink>
            </h3>
            <p class="card-description">
              {{
                survey.deadline
                  ? `截止于 ${dateTime(survey.deadline)}`
                  : survey.mode
                    ? '未设置截止时间'
                    : '完成编辑后，发布你的第一个版本。'
              }}
            </p>
          </div>
          <div class="card-meta">
            <span v-if="survey.taken_down" class="danger-text">已下架</span>
            <span>
              {{
                survey.mode === 'anonymous'
                  ? '匿名填写'
                  : survey.mode === 'login_once'
                    ? '登录限次'
                    : '填写模式待设置'
              }}
            </span>
            <span class="meta-dot">·</span>
            <span>修订 {{ survey.revision }}</span>
          </div>
          <div v-if="hasCardActions(survey)" class="card-actions">
            <button
              v-if="!survey.current_version_id"
              type="button"
              class="text-button danger-text"
              :disabled="busy"
              @click="confirmAction(survey, 'delete')"
            >
              删除草稿
            </button>
            <button
              v-if="survey.current_version_id"
              type="button"
              class="text-button"
              @click="copySurveyLink(survey)"
            >
              复制链接
            </button>
            <button
              v-if="survey.status === 'collecting'"
              type="button"
              class="text-button"
              :disabled="busy"
              @click="confirmAction(survey, 'close')"
            >
              关闭回收
            </button>
            <button
              v-if="survey.status === 'closed' && !survey.taken_down"
              type="button"
              class="text-button"
              :disabled="busy"
              @click="confirmAction(survey, 'resume')"
            >
              恢复回收
            </button>
            <button
              v-if="survey.current_version_id && survey.status !== 'archived'"
              type="button"
              class="text-button danger-text"
              :disabled="busy"
              @click="confirmAction(survey, 'archive')"
            >
              归档
            </button>
          </div>
          <div class="card-footer">
            <RouterLink
              class="draft-footer"
              :to="`/surveys/${survey.id}/${survey.current_version_id ? 'reports' : 'edit'}`"
            >
              {{ survey.current_version_id ? '查看答卷' : '继续编辑' }}
              <Icon name="arrow" />
            </RouterLink>
            <span class="card-updated">{{ dateTime(survey.updated_at) }}</span>
          </div>
        </article>
      </div>
      <div v-else class="empty-state">
        <Icon name="forms" />
        <h3>{{ keyword || filter ? '没有匹配的问卷' : '第一个好问题，从这里开始' }}</h3>
        <p>
          {{ keyword || filter ? '试试其他关键词或问卷状态。' : '创建问卷，收集你关心的答案。' }}
        </p>
        <button v-if="!keyword && !filter" class="button" @click="openCreate">新建问卷</button>
      </div>
      <Pagination
        :page="page"
        :total="list.data.value.total"
        :page-size="12"
        :disabled="list.loading.value"
        @change="page = $event"
      />
    </template>
  </section>
  <Modal v-if="creating" title="新建问卷" wide :dismissible="!busy" @close="creating = false">
    <form class="form-stack" @submit.prevent="create">
      <ErrorNotice :message="actionError" />
      <p class="subtle">从空白开始，或选择一个已配好规则的模板。</p>
      <SurveyTemplatePicker v-model="selectedTemplate" :disabled="busy" />
      <label class="field">
        问卷标题
        <input
          v-model="newTitle"
          autofocus
          required
          placeholder="这一次，你想了解什么？"
          :disabled="busy"
        />
      </label>
      <p class="hint">
        {{
          selectedTemplate
            ? '创建后可以修改模板，在规则工作区调整条件与流程。'
            : '创建后可添加题目、配置规则并选择回收方式。'
        }}
      </p>
      <div class="actions between">
        <button type="button" class="button" :disabled="busy" @click="creating = false">
          取消
        </button>
        <button class="button button-primary" :disabled="busy">
          {{ busy ? '正在创建…' : '创建草稿' }}
        </button>
      </div>
    </form>
  </Modal>
  <Modal
    v-if="pendingAction"
    :title="actionNames[pendingAction.action]"
    :dismissible="!busy"
    @close="closeAction"
  >
    <ErrorNotice :message="actionError" />
    <p class="subtle mb">{{ actionDescriptions[pendingAction.action] }}</p>
    <p class="subtle mb">问卷：{{ pendingAction.survey.title || '无标题问卷' }}</p>
    <div class="actions between">
      <button class="button" :disabled="busy" @click="closeAction">取消</button>
      <button
        class="button"
        :class="
          pendingAction.action === 'delete' ||
          pendingAction.action === 'archive' ||
          pendingAction.action === 'close'
            ? 'button-danger'
            : 'button-primary'
        "
        :disabled="busy"
        @click="performAction"
      >
        {{ busy ? '正在处理…' : `确认${actionNames[pendingAction.action]}` }}
      </button>
    </div>
  </Modal>
</template>
