<!-- 模板选择只改变选择状态，创建或应用由父级显式执行。 -->
<script setup lang="ts">
import { surveyTemplates } from '../survey-templates'
import type { SurveyTemplateId } from '../survey-templates'
import Icon from './Icon.vue'
const selected = defineModel<SurveyTemplateId | null>({ required: true })
withDefaults(defineProps<{ disabled?: boolean; allowBlank?: boolean }>(), { allowBlank: true })
</script>
<template>
  <div class="template-grid" role="group" aria-label="选择问卷起点">
    <button
      v-if="allowBlank"
      type="button"
      class="template-card"
      :aria-pressed="selected === null"
      :disabled="disabled"
      @click="selected = null"
    >
      <span class="template-icon"><Icon name="plus" /></span>
      <strong>从空白开始</strong>
      <span class="hint">自由组织题目，按需添加规则与跳题。</span>
      <span class="template-detail">空白画布 · 自由设计</span>
    </button>
    <button
      v-for="template in surveyTemplates"
      :key="template.id"
      type="button"
      class="template-card"
      :aria-pressed="selected === template.id"
      :disabled="disabled"
      @click="selected = template.id"
    >
      <span class="template-icon" :class="template.color"><Icon :name="template.icon" /></span>
      <strong>{{ template.title }}</strong>
      <span class="hint">{{ template.description }}</span>
      <span class="template-detail">{{ template.detail }}</span>
    </button>
  </div>
</template>
