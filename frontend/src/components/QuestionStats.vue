<!-- [入口在哪] 统计页逐题卡片；文本回答在卡片内按题分页拉取并带「查看答卷」。 -->
<!-- [用户在第 1 步后悔了怎么删/退] 本组件只读；返回筛选与重置筛选在报表页顶栏/筛选栏。 -->
<!-- [主提交通道在哪] 本组件无提交行为，唯一主操作「应用筛选」留在报表页筛选栏右侧。 -->
<!-- 统计分母来自当前版本的完整答卷；文本原文按题独立分页，不随统计响应一次性返回。 -->
<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { api, query } from '../api'
import { dateTime, exact } from '../format'
import { useResource } from '../resource'
import type { Page, QuestionStatistics, ReportSelection, TextResponse } from '../types'
import ErrorNotice from './ErrorNotice.vue'
import LoadingState from './LoadingState.vue'
import Pagination from './Pagination.vue'
const props = defineProps<{
  question: QuestionStatistics
  index: number
  surveyId: string
  selection: Pick<ReportSelection, 'version_id' | 'from' | 'to'>
}>()
defineEmits<{ receipt: [id: string] }>()
const page = ref(1)
const maximum = computed(() =>
  props.question.type === 'rating'
    ? props.question.distribution.reduce((max, item) => Math.max(max, item.count), 1)
    : 1,
)
const isText = computed(() => props.question.type === 'text' || props.question.type === 'textarea')
const responses = useResource(
  () =>
    isText.value
      ? api<Page<TextResponse>>(
          `/api/surveys/${props.surveyId}/questions/${props.question.question_id}/responses${query({
            ...props.selection,
            page: page.value,
            page_size: 20,
          })}`,
        )
      : Promise.resolve(null),
  [page, () => props.selection],
)
watch(
  () => props.selection,
  () => {
    page.value = 1
  },
)
function percent(value: number | null) {
  return value == null ? '暂无数据' : `${(value * 100).toFixed(1)}%`
}
function width(value: number | null) {
  return value == null ? '0%' : `${Math.max(0, Math.min(100, value * 100))}%`
}
</script>
<template>
  <section class="panel stat-question">
    <div class="panel-header">
      <h2>
        <span class="accent">{{ index + 1 }}.</span>
        {{ question.title }}
      </h2>
      <span class="hint">{{ question.answered_count }} 份有效回答</span>
    </div>
    <p class="hint mb">
      适用 {{ question.applicable_count }} 份 · 适用未答 {{ question.unanswered_count }} 份 ·
      规则跳过 {{ question.skipped_count }} 份（仅统计完整答卷）
    </p>
    <template v-if="question.type === 'single_choice' || question.type === 'multiple_choice'">
      <div class="distribution">
        <div v-for="option in question.options" :key="option.option_id">
          <div class="distribution-label">
            <span>{{ option.label }}</span>
            <span>{{ option.count }} 份 · {{ percent(option.proportion) }}</span>
          </div>
          <div class="bar-track">
            <div class="bar-fill" :style="{ width: width(option.proportion) }"></div>
          </div>
        </div>
      </div>
      <p v-if="question.proportion_note" class="hint mt">
        {{ question.proportion_note }}
      </p>
    </template>
    <template v-else-if="question.type === 'rating'">
      <div class="number-stats">
        <div>
          <span class="hint">平均评分</span>
          <strong>{{ exact(question.mean) }}</strong>
        </div>
        <div>
          <span class="hint">评分范围</span>
          <strong>{{ question.min }}–{{ question.max }}</strong>
        </div>
      </div>
      <div class="distribution">
        <div v-for="item in question.distribution" :key="item.value">
          <div class="distribution-label">
            <span>{{ item.value }} 分</span>
            <span>{{ item.count }} 份</span>
          </div>
          <div class="bar-track">
            <div class="bar-fill" :style="{ width: `${(item.count / maximum) * 100}%` }"></div>
          </div>
        </div>
      </div>
    </template>
    <div v-else-if="question.type === 'number'" class="number-stats">
      <div>
        <span class="hint">平均值</span>
        <strong>{{ exact(question.mean) }}</strong>
      </div>
      <div>
        <span class="hint">最小值</span>
        <strong>{{ exact(question.min) }}</strong>
      </div>
      <div>
        <span class="hint">最大值</span>
        <strong>{{ exact(question.max) }}</strong>
      </div>
    </div>
    <template v-else>
      <ErrorNotice :message="responses.error.value" retry @retry="responses.refresh" />
      <LoadingState v-if="responses.loading.value" />
      <template v-if="responses.data.value">
        <p v-if="!responses.data.value.items.length" class="hint">当前范围内暂无回答。</p>
        <div
          v-for="answer in responses.data.value.items"
          :key="answer.receipt_id"
          class="text-answer"
        >
          <div class="actions between">
            <span class="hint">{{ dateTime(answer.submitted_at) }}</span>
            <button class="text-button accent" @click="$emit('receipt', answer.receipt_id)">
              查看答卷
            </button>
          </div>
          <p class="plain-text">{{ answer.value }}</p>
        </div>
        <Pagination
          :page="page"
          :total="responses.data.value.total"
          :disabled="responses.loading.value"
          @change="page = $event"
        />
      </template>
    </template>
  </section>
</template>
