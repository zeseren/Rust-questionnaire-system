<!-- 当前草稿的试填只创建内存会话，修改配置后重建会话，不向服务端提交。 -->
<script setup lang="ts">
import { ref } from 'vue'
import { buildAnswers } from '../answers'
import type { Inputs } from '../answers'
import type { SurveyDocument } from '../types'
import FlowFields from './FlowFields.vue'
import SurveyFields from './SurveyFields.vue'
const props = defineProps<{ document: SurveyDocument; textPending: boolean }>()
const values = ref<Inputs>({})
const errors = ref<Record<string, string>>({})
const feedback = ref('')
const mobile = ref(false)
const generation = ref(0)
function reset() {
  values.value = {}
  errors.value = {}
  feedback.value = ''
  generation.value++
}
function clearErrors() {
  errors.value = {}
  feedback.value = ''
}
function validate() {
  const result = buildAnswers(props.document, values.value)
  errors.value = result.errors
  feedback.value = Object.keys(result.errors).length
    ? '请检查标记的题目。'
    : '试填校验通过，未提交答卷。'
}
</script>
<template>
  <section class="draft-preview">
    <div class="actions between mb">
      <div>
        <h2>试填当前草稿</h2>
        <p class="hint">体验显隐、必填与跳转，回答只保留在此处。</p>
      </div>
      <div class="actions">
        <div class="view-control" role="group" aria-label="预览宽度">
          <button type="button" class="button" :aria-pressed="!mobile" @click="mobile = false">
            桌面
          </button>
          <button type="button" class="button" :aria-pressed="mobile" @click="mobile = true">
            手机
          </button>
        </div>
        <button type="button" class="button" @click="reset">重新试填</button>
      </div>
    </div>
    <p v-if="textPending" class="notice mb">文本规则尚未应用，当前试填使用上一次应用的规则。</p>
    <div class="preview-paper" :class="{ 'preview-mobile': mobile }">
      <section class="panel public-survey-header">
        <p class="eyebrow">当前草稿 · 试填</p>
        <h1>{{ document.title || '无标题问卷' }}</h1>
        <p class="plain-text subtle">{{ document.description }}</p>
        <p class="plain-text hint mt">收集目的：{{ document.purpose || '尚未填写' }}</p>
      </section>
      <FlowFields
        v-if="document.logic && document.questions.length"
        :key="generation"
        :document="document"
        preview
        @submit="feedback = '试填校验通过，未提交答卷。'"
        @change="feedback = ''"
      />
      <template v-else-if="document.questions.length">
        <SurveyFields
          v-model="values"
          :document="document"
          :errors="errors"
          @change="clearErrors"
        />
        <div class="fill-footer">
          <span class="hint">回答仅用于试填。</span>
          <button type="button" class="button button-primary" @click="validate">
            检查填写结果
          </button>
        </div>
      </template>
      <div v-else class="empty-state">
        <h3>还没有题目</h3>
        <p>在题目设计中添加问题，即可在此试填。</p>
      </div>
      <p v-if="feedback" class="notice mt" role="status">{{ feedback }}</p>
    </div>
  </section>
</template>
