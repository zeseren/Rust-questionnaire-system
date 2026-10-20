<!-- [入口在哪] 问卷的答卷与统计页；顶栏常驻「刷新数据 / 查看问卷 / 导出 CSV」并标明当前筛选版本。 -->
<!-- [用户在第 1 步后悔了怎么删/退] 筛选栏与空结果区都有「重置筛选」，一次清空时间范围并恢复最新版本。 -->
<!-- [主提交通道在哪] 筛选栏右侧唯一主按钮「应用筛选」；长答卷详情弹窗底部固定「关闭」。 -->
<!-- 统计、答卷列表与 CSV 使用同一组已应用的版本及时间筛选。 -->
<script setup lang="ts">
import { computed, ref, shallowRef, watch } from 'vue'
import { RouterLink, useRoute } from 'vue-router'
import { api, download, message, query } from '../api'
import { answerText, dateTime, toTimestamp } from '../format'
import { useResource } from '../resource'
import { useNotice } from '../stores'
import type {
  Page,
  ReportSelection,
  ResponseDetail,
  ResponseSummary,
  Statistics,
  SurveyDetail,
  Version,
} from '../types'
import QuestionStats from '../components/QuestionStats.vue'
import ErrorNotice from '../components/ErrorNotice.vue'
import LoadingState from '../components/LoadingState.vue'
import Pagination from '../components/Pagination.vue'
import Modal from '../components/Modal.vue'
import Icon from '../components/Icon.vue'
const route = useRoute()
const id = String(route.params.id)
const notice = useNotice()
const tab = ref<'statistics' | 'responses'>('statistics')
const page = ref(1)
const selectedVersion = ref('')
const from = ref('')
const to = ref('')
const actionError = ref('')
const exporting = ref(false)
const selection = ref<Pick<ReportSelection, 'version_id' | 'from' | 'to'>>({
  version_id: '',
  from: null,
  to: null,
})
async function versions() {
  const items: Version[] = []
  let number = 1
  while (true) {
    const result = await api<Page<Version>>(
      `/api/surveys/${id}/versions${query({ page: number, page_size: 100 })}`,
    )
    items.push(...result.items)
    if (items.length >= result.total || !result.items.length) return items
    number += 1
  }
}
const metadata = useResource(async () => {
  const [survey, history] = await Promise.all([api<SurveyDetail>(`/api/surveys/${id}`), versions()])
  return { survey, history }
})
watch(metadata.data, (value) => {
  if (value?.survey.current_version_id) {
    selectedVersion.value = value.survey.current_version_id
    selection.value = { version_id: selectedVersion.value, from: null, to: null }
  }
})
const statistics = useResource(
  () =>
    selection.value.version_id
      ? api<Statistics>(`/api/surveys/${id}/statistics${query(selection.value)}`)
      : Promise.resolve(null),
  [selection],
)
const responses = useResource(
  () =>
    selection.value.version_id
      ? api<Page<ResponseSummary> & ReportSelection>(
          `/api/surveys/${id}/responses${query({ ...selection.value, page: page.value, page_size: 20 })}`,
        )
      : Promise.resolve(null),
  [selection, page],
)
const maximum = computed(
  () => statistics.data.value?.trend.reduce((max, day) => Math.max(max, day.count), 1) ?? 1,
)
const exportLabel = computed(() => {
  const number = metadata.data.value?.history.find(
    (version) => version.id === selection.value.version_id,
  )?.number
  return number == null ? '导出 CSV' : `导出 V${number} CSV`
})
const detailId = ref('')
const detail = shallowRef<ResponseDetail | null>(null)
const detailError = ref('')
const detailBusy = ref(false)
function refresh() {
  statistics.refresh()
  responses.refresh()
}
function apply() {
  actionError.value = ''
  const start = toTimestamp(from.value)
  const end = toTimestamp(to.value)
  if (
    (start && !Number.isFinite(Date.parse(start))) ||
    (end && !Number.isFinite(Date.parse(end))) ||
    (start && end && Date.parse(start) >= Date.parse(end))
  ) {
    actionError.value = '结束时间必须晚于开始时间。'
    return
  }
  page.value = 1
  selection.value = { version_id: selectedVersion.value, from: start, to: end }
}
function resetFilters() {
  actionError.value = ''
  from.value = ''
  to.value = ''
  selectedVersion.value = metadata.data.value?.survey.current_version_id ?? ''
  apply()
}
async function exportCsv() {
  exporting.value = true
  actionError.value = ''
  try {
    await download(
      `/api/surveys/${id}/export.csv${query(selection.value)}`,
      `survey-${id}-${selection.value.version_id}.csv`,
    )
    notice.show('CSV 文件已下载。')
  } catch (error) {
    actionError.value = message(error)
  } finally {
    exporting.value = false
  }
}
async function openReceipt(receipt: string) {
  if (detailBusy.value) return
  detailId.value = receipt
  detail.value = null
  detailBusy.value = true
  detailError.value = ''
  try {
    detail.value = await api<ResponseDetail>(`/api/surveys/${id}/responses/${receipt}`)
  } catch (error) {
    detailError.value = message(error)
  } finally {
    detailBusy.value = false
  }
}
</script>
<template>
  <div class="page-heading editor-heading">
    <div>
      <p class="eyebrow">答卷与统计</p>
      <h1>{{ metadata.data.value?.survey.draft.title || '回收数据' }}</h1>
      <p class="heading-note">按发布版本查看，每一份回答都有自己的来处。</p>
    </div>
    <div class="actions">
      <button class="button" :disabled="metadata.loading.value" @click="refresh">刷新数据</button>
      <RouterLink class="button" :to="`/surveys/${id}/edit`">查看问卷</RouterLink>
      <button class="button" :disabled="!selection.version_id || exporting" @click="exportCsv">
        <Icon name="collect" />
        {{ exporting ? '正在导出…' : exportLabel }}
      </button>
    </div>
  </div>
  <ErrorNotice :message="metadata.error.value" retry @retry="metadata.refresh" />
  <ErrorNotice :message="actionError" />
  <LoadingState v-if="metadata.loading.value" />
  <section
    v-if="metadata.data.value && !metadata.data.value.survey.current_version_id"
    class="empty-state"
  >
    <Icon name="chart" />
    <h3>发布之后，回答会汇聚在这里</h3>
    <p>这份问卷尚未发布，暂时没有答卷与统计。</p>
    <RouterLink class="button button-primary" :to="`/surveys/${id}/edit`">继续编辑问卷</RouterLink>
  </section>
  <template v-if="metadata.data.value?.survey.current_version_id">
    <form class="panel report-filter" @submit.prevent="apply">
      <label class="field">
        发布版本
        <select v-model="selectedVersion">
          <option
            v-for="version in metadata.data.value.history"
            :key="version.id"
            :value="version.id"
          >
            V{{ version.number }}{{ version.is_current ? '（当前）' : '' }} ·
            {{ dateTime(version.published_at) }}
          </option>
        </select>
      </label>
      <label class="field">
        开始时间（含）
        <input v-model="from" type="datetime-local" />
      </label>
      <label class="field">
        结束时间（不含）
        <input v-model="to" type="datetime-local" />
      </label>
      <button type="button" class="text-button danger-text" @click="resetFilters">重置筛选</button>
      <button class="button button-primary">应用筛选</button>
      <p class="hint" style="flex-basis: 100%">
        时间均为北京时间（Asia/Shanghai）。统计和导出使用已应用的筛选；刷新与稍后导出可能因新增答卷而不同。
      </p>
    </form>
    <div class="toolbar">
      <div class="tabs" role="group" aria-label="报表视图">
        <button class="tab" :aria-pressed="tab === 'statistics'" @click="tab = 'statistics'">
          统计概览
        </button>
        <button class="tab" :aria-pressed="tab === 'responses'" @click="tab = 'responses'">
          答卷明细
        </button>
      </div>
    </div>
    <template v-if="tab === 'statistics'">
      <ErrorNotice :message="statistics.error.value" retry @retry="statistics.refresh" />
      <LoadingState v-if="statistics.loading.value" />
      <template v-if="statistics.data.value">
        <section class="panel">
          <div class="report-summary">
            <div class="metric">
              <span class="hint">回收答卷</span>
              <strong class="metric-value">{{ statistics.data.value.total_count }}</strong>
            </div>
            <div class="metric">
              <span class="hint">统计版本</span>
              <strong class="metric-value">V{{ statistics.data.value.version_number }}</strong>
            </div>
            <div class="metric">
              <span class="hint">题目数量</span>
              <strong class="metric-value">{{ statistics.data.value.questions.length }}</strong>
            </div>
          </div>
          <h2>回收趋势</h2>
          <p class="hint mt">按北京时间自然日统计，未列出的日期回收数为 0。</p>
          <div
            v-if="statistics.data.value.trend.length"
            class="trend-chart"
            aria-label="每日回收数量"
          >
            <div
              v-for="day in statistics.data.value.trend"
              :key="day.date"
              class="trend-day"
              :title="`${day.date}：${day.count} 份`"
            >
              <span>{{ day.count }}</span>
              <i :style="{ height: `${(day.count / maximum) * 92}px` }" aria-hidden="true"></i>
              <span>{{ day.date.slice(5) }}</span>
            </div>
          </div>
          <div v-else class="actions mt">
            <span class="hint">当前版本和时间范围内暂无答卷。</span>
            <button type="button" class="text-button danger-text" @click="resetFilters">
              重置筛选
            </button>
          </div>
        </section>
        <QuestionStats
          v-for="(question, index) in statistics.data.value.questions"
          :key="`${statistics.data.value.version_id}-${question.question_id}`"
          :question="question"
          :index="index"
          @receipt="openReceipt"
        />
      </template>
    </template>
    <template v-else>
      <ErrorNotice :message="responses.error.value" retry @retry="responses.refresh" />
      <LoadingState v-if="responses.loading.value" />
      <template v-if="responses.data.value">
        <p class="hint mb">
          V{{ responses.data.value.version_number }} · {{ responses.data.value.total }} 份答卷 ·
          时间包含开始、不包含结束
        </p>
        <div v-if="responses.data.value.items.length" class="table-container">
          <table>
            <thead>
              <tr>
                <th>回执编号</th>
                <th>提交时间（北京时间）</th>
                <th>操作</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="response in responses.data.value.items" :key="response.receipt_id">
                <td>
                  <code>{{ response.receipt_id }}</code>
                </td>
                <td>{{ dateTime(response.submitted_at) }}</td>
                <td>
                  <button
                    class="text-button accent"
                    :disabled="detailBusy"
                    @click="openReceipt(response.receipt_id)"
                  >
                    查看答卷
                  </button>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
        <div v-else class="empty-state">
          <h3>当前范围内暂无答卷</h3>
          <p>可以调整版本或时间范围后再查看。</p>
          <button type="button" class="text-button danger-text" @click="resetFilters">
            重置筛选
          </button>
        </div>
        <Pagination :page="page" :total="responses.data.value.total" @change="page = $event" />
      </template>
    </template>
  </template>
  <Modal v-if="detailId" title="答卷详情" wide @close="detailId = ''">
    <LoadingState v-if="detailBusy" />
    <ErrorNotice :message="detailError" />
    <template v-if="detail">
      <p class="eyebrow">
        V{{ detail.version_number }} · {{ dateTime(detail.submitted_at) }}（北京时间）
      </p>
      <h2>{{ detail.document.title }}</h2>
      <code class="receipt">回执：{{ detail.receipt_id }}</code>
      <div
        v-for="(question, index) in detail.document.questions"
        :key="question.id"
        class="text-answer"
      >
        <h3>{{ index + 1 }}. {{ question.title }}</h3>
        <p class="plain-text">
          {{
            detail.question_states[question.id] === 'skipped'
              ? '规则跳过'
              : detail.question_states[question.id] === 'unanswered'
                ? '适用未答'
                : answerText(question, detail.answers[question.id])
          }}
        </p>
      </div>
    </template>
    <template #footer>
      <div class="actions between">
        <span class="hint">回执号：{{ detail?.receipt_id || detailId }}</span>
        <button type="button" class="button" @click="detailId = ''">关闭</button>
      </div>
    </template>
  </Modal>
</template>
