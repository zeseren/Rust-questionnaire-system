<!-- 搜索选择器保留失效引用的可见提示，并共享编辑器的只读状态。 -->
<script setup lang="ts">
import { computed, inject, ref } from 'vue'
import { ElOption, ElSelect } from 'element-plus'
import 'element-plus/es/components/select/style/css'
import 'element-plus/es/components/option/style/css'
import { editorDisabledKey } from '../editor-context'
const props = withDefaults(
  defineProps<{
    modelValue: string
    options: { value: string; label: string; disabled?: boolean }[]
    label: string
    placeholder?: string
    disabled?: boolean
  }>(),
  { placeholder: '搜索并选择' },
)
const emit = defineEmits<{ 'update:modelValue': [value: string] }>()
const locked = inject(editorDisabledKey, ref(false))
const missing = computed(
  () =>
    props.modelValue !== '' && !props.options.some((option) => option.value === props.modelValue),
)
function updateValue(value: unknown) {
  emit('update:modelValue', typeof value === 'string' ? value : '')
}
</script>
<template>
  <ElSelect
    class="editor-select"
    :model-value="modelValue"
    :aria-label="label"
    :placeholder="placeholder"
    :disabled="disabled || locked"
    teleported
    :empty-values="[null, undefined]"
    filterable
    no-match-text="没有匹配的选项"
    no-data-text="暂无可选项"
    popper-class="editor-select-popup"
    @update:model-value="updateValue"
  >
    <ElOption v-if="missing" :value="modelValue" label="当前引用不可用，请重新选择" disabled />
    <ElOption
      v-for="option in options"
      :key="option.value"
      :value="option.value"
      :label="option.label"
      :disabled="option.disabled"
    />
  </ElSelect>
</template>
