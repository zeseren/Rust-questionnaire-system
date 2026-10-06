<!-- 预览读取已保存草稿，只在当前页面校验，不产生正式答卷。 -->
<script setup lang="ts">
import { ref } from 'vue'
import { RouterLink, useRoute } from 'vue-router'
import { api } from '../api'
import { buildAnswers, type Inputs } from '../answers'
import { useResource } from '../resource'
import type { Preview } from '../types'
import SurveyFields from '../components/SurveyFields.vue'
import ErrorNotice from '../components/ErrorNotice.vue'
import LoadingState from '../components/LoadingState.vue'
const route = useRoute()
const preview = useResource(() => api<Preview>(`/api/surveys/${route.params.id}/preview`))
const values = ref<Inputs>({})
const errors = ref<Record<string, string>>({})
const mobile = ref(false)
const feedback = ref('')
function validate() {
  if (preview.data.value?.document.logic) return
  if (!preview.data.value?.publishable) {
    feedback.value = '请先修正发布校验问题，再体验填写校验。'
    return
  }
  const result = buildAnswers(preview.data.value.document, values.value)
  errors.value = result.errors
  feedback.value = Object.keys(result.errors).length
    ? '请补全下方标记的问题。'
    : '填写校验通过。这是预览，未提交正式答卷。'
}
</script>
<template>
  <div class="public-container">
    <div class="actions between mb">
      <RouterLink class="text-button" :to="`/surveys/${route.params.id}/edit`">
        ← 返回编辑
      </RouterLink>
      <div class="view-control">
        <button class="button" :aria-pressed="!mobile" @click="mobile = false">桌面</button>
        <button class="button" :aria-pressed="mobile" @click="mobile = true">手机</button>
      </div>
    </div>
    <p class="notice">草稿预览：展示服务器已保存的内容，不会收集正式答卷。</p>
    <ErrorNotice :message="preview.error.value" retry @retry="preview.refresh" />
    <LoadingState v-if="preview.loading.value" />
    <template v-if="preview.data.value">
      <ErrorNotice :message="preview.data.value.validation_error?.message || ''" />
      <div :class="{ 'preview-mobile': mobile }">
        <section class="panel public-survey-header">
          <p class="eyebrow">草稿 · 修订 {{ preview.data.value.revision }}</p>
          <h1>{{ preview.data.value.document.title || '无标题问卷' }}</h1>
          <p class="plain-text subtle">{{ preview.data.value.document.description }}</p>
          <p class="plain-text hint mt">
            收集目的：{{ preview.data.value.document.purpose || '尚未填写' }}
          </p>
        </section>
        <p v-if="preview.data.value.document.logic" class="notice mt">
          此问卷的分步预览暂未开放，请先返回编辑核对题目。
        </p>
        <SurveyFields
          v-else
          v-model="values"
          :document="preview.data.value.document"
          :errors="errors"
        />
        <p v-if="feedback" class="notice mt" role="status">{{ feedback }}</p>
        <div v-if="!preview.data.value.document.logic" class="fill-footer">
          <span class="hint">此处的回答仅用于预览。</span>
          <button class="button button-primary" @click="validate">体验填写校验</button>
        </div>
      </div>
    </template>
  </div>
</template>
