<!-- [入口在哪] 编辑器「试填预览」工作区直接渲染当前草稿，无需先保存。 -->
<!-- [用户在第 1 步后悔了怎么删/退] 底部固定栏最左侧常驻「重新预览」，随时清空本次试填；正文可切回其它工作区或顶部返回列表，不会误提交。 -->
<!-- [主提交通道在哪] 同屏顶部编辑器固定栏的「发布问卷」是唯一主按钮；本页右侧「检查填写结果」只做本地校验，降级为次级按钮。 -->
<!-- 当前草稿的试填只创建内存会话，修改配置后重建会话，不向服务端提交。 -->
<script setup lang="ts">
import { ref } from 'vue'
import type { SurveyDocument } from '../types'
import FlowFields from './FlowFields.vue'
defineProps<{ document: SurveyDocument; textPending: boolean }>()
const feedback = ref('')
const mobile = ref(false)
</script>
<template>
  <section class="draft-preview">
    <div class="actions between mb">
      <div>
        <h2>试填当前草稿</h2>
        <p class="hint">体验显隐、必填与跳转，回答只保留在此处。</p>
      </div>
      <div class="view-control" role="group" aria-label="预览宽度">
        <button type="button" class="button" :aria-pressed="!mobile" @click="mobile = false">
          桌面
        </button>
        <button type="button" class="button" :aria-pressed="mobile" @click="mobile = true">
          手机
        </button>
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
        v-if="document.questions.length"
        :document="document"
        preview
        @submit="feedback = '试填校验通过，未提交答卷。'"
        @change="feedback = ''"
      />
      <div v-else class="empty-state">
        <h3>还没有题目</h3>
        <p>在题目设计中添加问题，即可在此试填。</p>
      </div>
      <p v-if="feedback" class="notice mt" role="status">{{ feedback }}</p>
    </div>
  </section>
</template>
