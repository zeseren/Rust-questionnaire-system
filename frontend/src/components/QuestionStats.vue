<!-- [入口在哪] 统计页逐题卡片；文本回答在卡片内直接列出并带「查看答卷」。 -->
<!-- [用户在第 1 步后悔了怎么删/退] 本组件只读；返回筛选与重置筛选在报表页顶栏/筛选栏。 -->
<!-- [主提交通道在哪] 本组件无提交行为，唯一主操作「应用筛选」留在报表页筛选栏右侧。 -->
<!-- 统计分母来自当前版本的有效回答数，空均值与空占比不显示为零。 -->
<script setup lang="ts">
import { computed, ref } from 'vue'
import { dateTime, exact } from '../format'
import type { QuestionStatistics } from '../types'
const props = defineProps<{ question: QuestionStatistics; index: number }>()
defineEmits<{ receipt: [id: string] }>()
const limit = ref(20)
const maximum = computed(() =>
  props.question.type === 'rating'
    ? props.question.distribution.reduce((max, item) => Math.max(max, item.count), 1)
    : 1,
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
      规则跳过 {{ question.skipped_count }} 份
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
    <template v-else-if="question.type === 'text' || question.type === 'textarea'">
      <p v-if="!question.answers.length" class="hint">当前范围内暂无回答。</p>
      <div
        v-for="answer in question.answers.slice(0, limit)"
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
      <button v-if="limit < question.answers.length" class="button mt" @click="limit += 20">
        查看更多回答（{{ Math.min(limit, question.answers.length) }} /
        {{ question.answers.length }}）
      </button>
    </template>
  </section>
</template>
