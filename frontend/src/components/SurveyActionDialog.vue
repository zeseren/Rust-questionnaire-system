<!-- [入口在哪] 编辑器顶部栏与问卷列表卡片触发对应的生命周期操作。 -->
<!-- [用户在第 1 步后悔了怎么删/退] 标题栏关闭、Esc、点击遮罩或底栏「取消」随时退出；处理中不可关闭。 -->
<!-- [主提交通道在哪] 底栏右侧危险/主按钮确认当前操作。 -->
<!-- 编辑器与列表共用同一段说明、危险样式和按钮文案，差异内容由默认插槽补充。 -->
<script setup lang="ts">
import ErrorNotice from './ErrorNotice.vue'
import Modal from './Modal.vue'
import {
  SURVEY_ACTION_DESCRIPTIONS,
  SURVEY_ACTION_NAMES,
  type SurveyLifecycleAction,
} from '../survey-actions'
defineProps<{
  action: SurveyLifecycleAction
  busy: boolean
  error?: string
}>()
defineEmits<{ confirm: []; close: [] }>()
</script>
<template>
  <Modal :title="SURVEY_ACTION_NAMES[action]" :dismissible="!busy" @close="$emit('close')">
    <ErrorNotice v-if="error" :message="error" />
    <p class="subtle mb">{{ SURVEY_ACTION_DESCRIPTIONS[action] }}</p>
    <slot />
    <div class="actions between">
      <button class="button" :disabled="busy" @click="$emit('close')">取消</button>
      <button
        class="button"
        :class="
          action === 'delete' || action === 'archive' || action === 'close'
            ? 'button-danger'
            : 'button-primary'
        "
        :disabled="busy"
        @click="$emit('confirm')"
      >
        {{ busy ? '正在处理…' : `确认${SURVEY_ACTION_NAMES[action]}` }}
      </button>
    </div>
  </Modal>
</template>
