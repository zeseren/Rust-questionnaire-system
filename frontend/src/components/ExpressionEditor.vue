<!-- [入口在哪] 变量、规则条件或流程分支切换到「高级表达式」后，在此逐层编辑 AST。 -->
<!-- [用户在第 1 步后悔了怎么删/退] 每个子条件右侧常驻低饱和「删除条件」幽灵按钮，悬停变红；切换顶层表达式类型即可整体重建。 -->
<!-- [主提交通道在哪] 表达式编辑不设主按钮，修改即时写回模型；保存与发布由编辑器顶部固定栏统一承担。 -->
<!-- 每次编辑替换 AST 节点，数字常量保留原文，引用始终绑定稳定身份。 -->
<script setup lang="ts">
import { computed, inject, ref } from 'vue'
import type { ComputedVariable, Question } from '../types'
import { editorDisabledKey } from '../editor-context'
import SearchSelect from './SearchSelect.vue'
const props = withDefaults(
  defineProps<{
    modelValue: unknown
    questions: Question[]
    variables: ComputedVariable[]
    depth?: number
  }>(),
  { depth: 1 },
)
const emit = defineEmits<{ 'update:modelValue': [value: unknown] }>()
const locked = inject(editorDisabledKey, ref(false))
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
function isUnknownArray(value: unknown): value is readonly unknown[] {
  return Array.isArray(value)
}
function asList(value: unknown): readonly unknown[] {
  return isUnknownArray(value) ? value : []
}
const node = computed<Record<string, unknown>>(() =>
  isRecord(props.modelValue) ? props.modelValue : { op: 'boolean', value: true },
)
const ops = {
  boolean: '布尔常量',
  number: '数字常量',
  text: '文本常量',
  answer: '题目答案',
  answered: '题目已回答',
  selected: '选中选项',
  contains_any: '包含任一选项',
  contains_all: '包含全部选项',
  variable: '计算变量',
  not: '取反',
  all: '全部成立',
  any: '任一成立',
  compare: '比较',
  coalesce: '缺失时替代',
  length: '长度',
  arithmetic: '算术',
}
const children = computed<readonly (readonly [string, string])[]>(() => {
  switch (node.value.op) {
    case 'not':
      return [['expression', '条件'] as const]
    case 'length':
      return [['value', '值'] as const]
    case 'coalesce':
      return [['value', '值'] as const, ['fallback', '缺失时使用'] as const]
    case 'compare':
    case 'arithmetic':
      return [['left', '左值'] as const, ['right', '右值'] as const]
    default:
      return []
  }
})
const list = computed<readonly unknown[]>(() => asList(node.value.expressions))
const selectedOptionIds = computed<string[]>(() =>
  asList(node.value.option_ids).filter((item): item is string => typeof item === 'string'),
)
const question = computed(() => props.questions.find((q) => q.id === node.value.question_id))
const options = computed(() =>
  question.value && 'options' in question.value ? question.value.options : [],
)
function update(key: string, value: unknown) {
  emit('update:modelValue', { ...node.value, [key]: value })
}
const referenceQuestions = computed(() =>
  ['selected', 'contains_any', 'contains_all'].includes(String(node.value.op))
    ? props.questions.filter((question) => 'options' in question)
    : props.questions,
)
function changeQuestion(id: string) {
  const question = props.questions.find((question) => question.id === id)
  const fields =
    node.value.op === 'selected'
      ? { option_id: question && 'options' in question ? (question.options[0]?.id ?? '') : '' }
      : node.value.op === 'contains_any' || node.value.op === 'contains_all'
        ? { option_ids: [] }
        : {}
  emit('update:modelValue', { ...node.value, question_id: id, ...fields })
}
function switchOp(op: string) {
  const literal = { op: 'boolean', value: true }
  const number = { op: 'number', value: '0' }
  const selectedQuestion = props.questions.find((question) => 'options' in question)
  const question_id = props.questions[0]?.id ?? ''
  let fields: Record<string, unknown>
  switch (op) {
    case 'boolean':
      fields = { value: true }
      break
    case 'number':
      fields = { value: '0' }
      break
    case 'text':
      fields = { value: '' }
      break
    case 'answer':
    case 'answered':
      fields = { question_id }
      break
    case 'selected':
      fields = {
        question_id: selectedQuestion?.id ?? '',
        option_id:
          selectedQuestion && 'options' in selectedQuestion
            ? (selectedQuestion.options[0]?.id ?? '')
            : '',
      }
      break
    case 'contains_any':
    case 'contains_all':
      fields = { question_id: selectedQuestion?.id ?? '', option_ids: [] }
      break
    case 'variable':
      fields = { variable_id: props.variables[0]?.id ?? '' }
      break
    case 'not':
      fields = { expression: literal }
      break
    case 'all':
    case 'any':
      fields = { expressions: [literal] }
      break
    case 'compare':
      fields = { comparison: 'eq', left: number, right: number }
      break
    case 'arithmetic':
      fields = { operator: 'add', left: number, right: number }
      break
    case 'coalesce':
      fields = { value: { op: 'answer', question_id }, fallback: number }
      break
    default:
      fields = { value: { op: 'answer', question_id } }
  }
  emit('update:modelValue', { op, ...fields })
}
function editList(index: number, value: unknown) {
  update(
    'expressions',
    list.value.map((item, i) => (i === index ? value : item)),
  )
}
function toggleOption(id: string) {
  const ids = selectedOptionIds.value
  update('option_ids', ids.includes(id) ? ids.filter((value) => value !== id) : [...ids, id])
}
function selectValue(event: Event) {
  return event.currentTarget instanceof HTMLSelectElement ? event.currentTarget.value : ''
}
function inputValue(event: Event) {
  return event.currentTarget instanceof HTMLInputElement ? event.currentTarget.value : ''
}
</script>
<template>
  <div class="expression-editor">
    <label class="field">
      表达式
      <select
        aria-label="表达式"
        :value="node.op"
        :disabled="locked"
        @change="switchOp(selectValue($event))"
      >
        <option v-for="(label, op) in ops" :key="op" :value="op">{{ label }}</option>
      </select>
    </label>
    <label v-if="node.op === 'boolean'" class="field">
      值
      <select
        :value="String(node.value)"
        :disabled="locked"
        @change="update('value', selectValue($event) === 'true')"
      >
        <option value="true">成立</option>
        <option value="false">不成立</option>
      </select>
    </label>
    <label v-if="node.op === 'number' || node.op === 'text'" class="field">
      {{ node.op === 'number' ? '数字原文' : '文本' }}
      <input :value="node.value" :disabled="locked" @input="update('value', inputValue($event))" />
    </label>
    <label v-if="'question_id' in node" class="field">
      引用题目
      <SearchSelect
        :model-value="String(node.question_id ?? '')"
        label="引用题目"
        :options="
          referenceQuestions.map((question) => ({
            value: question.id,
            label: `${questions.findIndex((item) => item.id === question.id) + 1}. ${question.title || '未命名题目'}`,
          }))
        "
        @update:model-value="changeQuestion"
      />
    </label>
    <label v-if="node.op === 'selected'" class="field">
      选项
      <SearchSelect
        :model-value="String(node.option_id ?? '')"
        label="引用选项"
        :options="
          options.map((option) => ({ value: option.id, label: option.label || '未命名选项' }))
        "
        @update:model-value="update('option_id', $event)"
      />
    </label>
    <div v-if="node.op === 'contains_any' || node.op === 'contains_all'" class="form-stack">
      <label v-for="option in options" :key="option.id">
        <input
          type="checkbox"
          :checked="selectedOptionIds.includes(option.id)"
          :disabled="locked"
          @change="toggleOption(option.id)"
        />
        {{ option.label }}
      </label>
      <button
        v-for="id in selectedOptionIds.filter((id) => !options.some((o) => o.id === id))"
        :key="id"
        type="button"
        class="text-button danger-text"
        :disabled="locked"
        @click="toggleOption(id)"
      >
        移除失效选项 {{ id }}
      </button>
    </div>
    <label v-if="node.op === 'variable'" class="field">
      引用变量
      <SearchSelect
        :model-value="String(node.variable_id ?? '')"
        label="引用变量"
        :options="
          variables.map((variable, index) => ({
            value: variable.id,
            label: `变量 ${index + 1} · ${variable.value_type}`,
          }))
        "
        @update:model-value="update('variable_id', $event)"
      />
    </label>
    <label v-if="node.op === 'compare'" class="field">
      比较方式
      <select
        :value="node.comparison"
        :disabled="locked"
        @change="update('comparison', selectValue($event))"
      >
        <option
          v-for="(label, key) in {
            eq: '等于',
            ne: '不等于',
            lt: '小于',
            le: '小于等于',
            gt: '大于',
            ge: '大于等于',
          }"
          :key="key"
          :value="key"
        >
          {{ label }}
        </option>
      </select>
    </label>
    <label v-if="node.op === 'arithmetic'" class="field">
      运算
      <select
        :value="node.operator"
        :disabled="locked"
        @change="update('operator', selectValue($event))"
      >
        <option
          v-for="(label, key) in { add: '加', subtract: '减', multiply: '乘', divide: '除' }"
          :key="key"
          :value="key"
        >
          {{ label }}
        </option>
      </select>
    </label>
    <template v-if="depth < 16">
      <div v-for="[key, label] in children" :key="key" class="expression-child">
        <p class="hint">{{ label }}</p>
        <ExpressionEditor
          :model-value="node[key]"
          :questions="questions"
          :variables="variables"
          :depth="depth + 1"
          @update:model-value="update(key, $event)"
        />
      </div>
      <template v-if="node.op === 'all' || node.op === 'any'">
        <div v-for="(item, index) in list" :key="index" class="expression-child">
          <div class="actions between">
            <span class="hint">条件 {{ index + 1 }}</span>
            <button
              type="button"
              class="text-button danger-text"
              :disabled="locked"
              @click="
                update(
                  'expressions',
                  list.filter((_, i) => i !== index),
                )
              "
            >
              删除条件
            </button>
          </div>
          <ExpressionEditor
            :model-value="item"
            :questions="questions"
            :variables="variables"
            :depth="depth + 1"
            @update:model-value="editList(index, $event)"
          />
        </div>
        <button
          type="button"
          class="button"
          :disabled="locked || list.length >= 256"
          @click="update('expressions', [...list, { op: 'boolean', value: true }])"
        >
          添加条件
        </button>
      </template>
    </template>
    <p v-else-if="children.length || list.length" class="hint">已到最大嵌套层数，请简化表达式。</p>
  </div>
</template>
