<!-- 每次编辑替换 AST 节点，数字常量保留原文，引用始终绑定稳定身份。 -->
<script setup lang="ts">
import { computed } from 'vue'
import type { Question } from '../types'
import type { ComputedVariable } from '../document-logic'
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
const node = computed(
  () => (props.modelValue ?? { op: 'boolean', value: true }) as Record<string, unknown>,
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
const children = computed(() => {
  switch (node.value.op) {
    case 'not':
      return [['expression', '条件']]
    case 'length':
      return [['value', '值']]
    case 'coalesce':
      return [
        ['value', '值'],
        ['fallback', '缺失时使用'],
      ]
    case 'compare':
    case 'arithmetic':
      return [
        ['left', '左值'],
        ['right', '右值'],
      ]
    default:
      return []
  }
})
const list = computed(() => (node.value.expressions ?? []) as unknown[])
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
  const ids = (node.value.option_ids ?? []) as string[]
  update('option_ids', ids.includes(id) ? ids.filter((value) => value !== id) : [...ids, id])
}
</script>
<template>
  <div class="expression-editor">
    <label class="field">
      表达式
      <select
        aria-label="表达式"
        :value="node.op"
        @change="switchOp(($event.target as HTMLSelectElement).value)"
      >
        <option v-for="(label, op) in ops" :key="op" :value="op">{{ label }}</option>
      </select>
    </label>
    <label v-if="node.op === 'boolean'" class="field">
      值
      <select
        :value="String(node.value)"
        @change="update('value', ($event.target as HTMLSelectElement).value === 'true')"
      >
        <option value="true">成立</option>
        <option value="false">不成立</option>
      </select>
    </label>
    <label v-if="node.op === 'number' || node.op === 'text'" class="field">
      {{ node.op === 'number' ? '数字原文' : '文本' }}
      <input
        :value="node.value"
        @input="update('value', ($event.target as HTMLInputElement).value)"
      />
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
          :checked="(node.option_ids as string[]).includes(option.id)"
          @change="toggleOption(option.id)"
        />
        {{ option.label }}
      </label>
      <button
        v-for="id in (node.option_ids as string[]).filter(
          (id) => !options.some((o) => o.id === id),
        )"
        :key="id"
        type="button"
        class="text-button danger-text"
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
        @change="update('comparison', ($event.target as HTMLSelectElement).value)"
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
        @change="update('operator', ($event.target as HTMLSelectElement).value)"
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
          :model-value="node[key!]"
          :questions="questions"
          :variables="variables"
          :depth="depth + 1"
          @update:model-value="update(key!, $event)"
        />
      </div>
      <template v-if="node.op === 'all' || node.op === 'any'">
        <div v-for="(item, index) in list" :key="index" class="expression-child">
          <div class="actions between">
            <span class="hint">条件 {{ index + 1 }}</span>
            <button
              type="button"
              class="text-button"
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
          :disabled="list.length >= 256"
          @click="update('expressions', [...list, { op: 'boolean', value: true }])"
        >
          添加条件
        </button>
      </template>
    </template>
    <p v-else-if="children.length || list.length" class="hint">已到最大嵌套层数，请简化表达式。</p>
  </div>
</template>
