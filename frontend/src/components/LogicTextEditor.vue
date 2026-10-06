<!-- 未应用的文本单独保留，语法或语义失败不能覆盖当前规则。 -->
<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import type { DocumentLogic } from '../document-logic'
import type { Question } from '../types'
import { formatLogicText, LogicTextError, parseLogicText } from '../logic-text'
const logic = defineModel<DocumentLogic>({ required: true })
const props = defineProps<{ questions: Question[] }>()
const emit = defineEmits<{ pending: [value: boolean] }>()
const text = ref(''),
  baseline = ref(''),
  error = ref('')
const source = ref<HTMLTextAreaElement>()
const pending = computed(() => text.value !== baseline.value)
watch(pending, (value) => emit('pending', value), { immediate: true })
function generate() {
  try {
    text.value = formatLogicText(logic.value, props.questions)
    baseline.value = text.value
    error.value = ''
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : '无法生成文本，请先修复可视化配置。'
  }
}
async function apply() {
  try {
    logic.value = parseLogicText(text.value, props.questions)
    baseline.value = text.value
    error.value = ''
  } catch (reason) {
    error.value = reason instanceof Error ? reason.message : '文本无法应用'
    if (reason instanceof LogicTextError) {
      error.value += ` · ${reason.path}`
      await nextTick()
      const lines = text.value.split('\n')
      const offset =
        lines.slice(0, reason.line - 1).reduce((n, line) => n + line.length + 1, 0) +
        [...(lines[reason.line - 1] ?? '')].slice(0, reason.column - 1).join('').length
      source.value?.focus()
      source.value?.setSelectionRange(offset, offset)
    }
  }
}
generate()
onBeforeUnmount(() => emit('pending', false))
</script>
<template>
  <div class="form-stack">
    <p class="hint">
      别名通过 bind
      绑定题目、选项、变量和节点。修改文本后先应用，再保存草稿；应用会检查全部类型、依赖和流程。
    </p>
    <details class="hint">
      <summary>语法提示</summary>
      <pre>
let v1: boolean = answered(q1);
visible q2 = variable(v1);
required q2 = selected(q1, q1_o1);
validate q2 = answer(q2) &gt;= 0;
page n1 [q1] -&gt; n2;
branch n2 {
  when selected(q1, q1_o1) -&gt; n3;
  else -&gt; n4;
}
end n4 = complete;</pre
      >
      <p>
        新变量和节点需要先声明 UUID 绑定。题目与选项在上方题目编辑区建立。筛选结束 screen_out
        仅可预览。
      </p>
    </details>
    <label class="field">
      文本 DSL
      <textarea ref="source" v-model="text" class="dsl-source" spellcheck="false" wrap="off" />
    </label>
    <p v-if="error" class="notice" role="alert">{{ error }}</p>
    <p v-else class="hint" role="status">
      {{ pending ? '文本尚未应用，当前规则仍保持原样。' : '文本已与当前规则同步。' }}
    </p>
    <div class="actions">
      <button type="button" class="button button-primary" @click="apply">应用文本规则</button>
      <button type="button" class="button" @click="generate">恢复当前规则文本</button>
    </div>
  </div>
</template>
