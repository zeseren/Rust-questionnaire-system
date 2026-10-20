<!-- [入口在哪] 编辑器固定栏「预览已保存草稿」或问卷卡片「查看答卷」进入，只读服务器已保存内容。 -->
<!-- [用户在第 1 步后悔了怎么删/退] 顶部固定工具栏左侧常驻「← 返回编辑」，底部固定栏左侧同样常驻返回入口，任何位置都能退出预览。 -->
<!-- [主提交通道在哪] 预览不产生正式答卷；顶部右侧「返回列表」是次级出口，底部固定栏右侧「体验填写校验」是页面唯一主按钮。 -->
<!-- 预览读取已保存草稿，只在当前页面校验，不产生正式答卷。 -->
<script setup lang="ts">
import { computed, ref } from 'vue'
import { RouterLink, useRoute } from 'vue-router'
import { api } from '../api'
import { buildAnswers, isAnswered, type Inputs } from '../answers'
import { useResource } from '../resource'
import type { Preview } from '../types'
import SurveyFields from '../components/SurveyFields.vue'
import FlowFields from '../components/FlowFields.vue'
import ErrorNotice from '../components/ErrorNotice.vue'
import LoadingState from '../components/LoadingState.vue'
const route = useRoute()
const preview = useResource(() => api<Preview>(`/api/surveys/${route.params.id}/preview`))
const values = ref<Inputs>({})
const errors = ref<Record<string, string>>({})
const mobile = ref(false)
const feedback = ref('')
const answeredCount = computed(() => Object.values(values.value).filter(isAnswered).length)
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
    <div class="preview-toolbar mb">
      <RouterLink class="text-button" :to="`/surveys/${route.params.id}/edit`">
        ← 返回编辑
      </RouterLink>
      <div class="actions">
        <div class="view-control">
          <button class="button" :aria-pressed="!mobile" @click="mobile = false">桌面</button>
          <button class="button" :aria-pressed="mobile" @click="mobile = true">手机</button>
        </div>
        <RouterLink class="button" to="/surveys">返回列表</RouterLink>
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
        <FlowFields
          v-if="preview.data.value.document.logic"
          :document="preview.data.value.document"
          preview
          @submit="feedback = '填写校验通过。这是预览，未提交正式答卷。'"
          @change="feedback = ''"
        />
        <SurveyFields
          v-else
          v-model="values"
          :document="preview.data.value.document"
          :errors="errors"
        />
        <p v-if="feedback" class="notice mt" role="status">{{ feedback }}</p>
        <div v-if="!preview.data.value.document.logic" class="fill-footer">
          <div class="fill-footer-left">
            <RouterLink class="text-button" :to="`/surveys/${route.params.id}/edit`">
              ← 返回编辑
            </RouterLink>
            <span class="hint">已回答 {{ answeredCount }} 题，回答仅用于预览。</span>
          </div>
          <div class="fill-footer-right">
            <button class="button button-primary" @click="validate">体验填写校验</button>
          </div>
        </div>
      </div>
    </template>
  </div>
</template>
