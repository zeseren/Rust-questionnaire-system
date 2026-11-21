<!-- [入口在哪] 变量、规则条件或流程分支切换到「高级表达式」后，在此逐层编辑 AST。 -->
<!-- [用户在第 1 步后悔了怎么删/退] 每个子条件右侧常驻低饱和「删除条件」幽灵按钮，悬停变红；切换顶层表达式类型即可整体重建。 -->
<!-- [主提交通道在哪] 表达式编辑不设主按钮，修改即时写回模型；保存与发布由编辑器顶部固定栏统一承担。 -->
<!-- 每次编辑替换 AST 节点，数字常量保留原文，引用始终绑定稳定身份。 -->
<script setup lang="ts">
import { computed, inject, ref } from 'vue'
import type { Choice, ComputedVariable, Question } from '../types'
import { comparisonNames, inputValue, selectValue } from '../condition-builder'
import { editorDisabledKey } from '../editor-context'
import { numberedQuestionLabel } from '../flow-tools'
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
  option: '选项常量',
  contains_any: '包含任一选项',
  contains_all: '包含全部选项',
  variable: '计算变量',
  not: '取反',
  all: '全部成立',
  any: '任一成立',
  compare: '比较',
  coalesce: '缺失时替代',
  length: '长度',
  if: '条件选择',
  negate: '取负',
  contains: '包含子串',
  starts_with: '前缀匹配',
  matches: '正则匹配',
  arithmetic: '算术',
  round: '四舍五入',
  floor: '向下取整',
  ceil: '向上取整',
}
// 高级编辑沿用原有「小于等于/大于等于」措辞；其余标签复用简洁条件的 comparisonNames。
const comparisonLabels = {
  eq: comparisonNames.eq,
  ne: comparisonNames.ne,
  lt: comparisonNames.lt,
  le: '小于等于',
  gt: comparisonNames.gt,
  ge: '大于等于',
} as const
const children = computed<readonly (readonly [string, string])[]>(() => {
  switch (node.value.op) {
    case 'not':
      return [['expression', '条件'] as const]
    case 'length':
    case 'negate':
    case 'round':
    case 'floor':
    case 'ceil':
      return [['value', '值'] as const]
    case 'coalesce':
      return [['value', '值'] as const, ['fallback', '缺失时使用'] as const]
    case 'if':
      return [
        ['condition', '条件'] as const,
        ['then_branch', '条件成立时'] as const,
        ['else_branch', '条件不成立时'] as const,
      ]
    case 'contains':
      return [['text', '文本'] as const, ['substring', '包含的子串'] as const]
    case 'starts_with':
      return [['text', '文本'] as const, ['prefix', '前缀'] as const]
    case 'matches':
      return [['text', '文本'] as const]
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
const choiceQuestions = computed(() =>
  props.questions.filter(
    (question): question is Question & { options: Choice[] } => 'options' in question,
  ),
)
const optionOwner = computed(() =>
  choiceQuestions.value.find((question) =>
    question.options.some((option) => option.id === node.value.option_id),
  ),
)
const optionCandidates = computed(() => optionOwner.value?.options ?? [])
// digits 是 JSON 整数：输入框越界或留空时收敛到最近的合法值，避免写出非法 AST。
function digitsValue(event: Event): number {
  const raw = inputValue(event)
  const parsed = Number(raw)
  if (raw.trim() === '' || !Number.isFinite(parsed)) return 0
  return Math.min(6, Math.max(0, Math.trunc(parsed)))
}
function changeOptionQuestion(id: string) {
  const target = choiceQuestions.value.find((question) => question.id === id)
  update('option_id', target?.options[0]?.id ?? '')
}
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
  const selectedQuestion = choiceQuestions.value[0]
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
        option_id: selectedQuestion?.options[0]?.id ?? '',
      }
      break
    case 'contains_any':
    case 'contains_all':
      fields = { question_id: selectedQuestion?.id ?? '', option_ids: [] }
      break
    case 'variable':
      fields = { variable_id: props.variables[0]?.id ?? '' }
      break
    case 'option':
      fields = { option_id: selectedQuestion?.options[0]?.id ?? '' }
      break
    case 'matches':
      fields = { text: { op: 'answer', question_id }, pattern: '' }
      break
    case 'round':
    case 'floor':
    case 'ceil':
      fields = { value: number, digits: 0 }
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
    case 'if':
      fields = { condition: literal, then_branch: number, else_branch: number }
      break
    case 'negate':
      fields = { value: number }
      break
    case 'contains':
    case 'starts_with':
      fields = {
        text: { op: 'answer', question_id },
        [op === 'contains' ? 'substring' : 'prefix']: { op: 'text', value: '' },
      }
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
            label: numberedQuestionLabel(
              question,
              questions.findIndex((item) => item.id === question.id),
            ),
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
    <template v-if="node.op === 'option'">
      <label class="field">
        所属题目
        <SearchSelect
          :model-value="optionOwner?.id ?? ''"
          label="所属题目"
          :options="
            choiceQuestions.map((question) => ({
              value: question.id,
              label: numberedQuestionLabel(
                question,
                questions.findIndex((item) => item.id === question.id),
              ),
            }))
          "
          @update:model-value="changeOptionQuestion"
        />
      </label>
      <label class="field">
        选项
        <SearchSelect
          :model-value="String(node.option_id ?? '')"
          label="选项"
          :options="
            optionCandidates.map((option) => ({
              value: option.id,
              label: option.label || '未命名选项',
            }))
          "
          @update:model-value="update('option_id', $event)"
        />
      </label>
    </template>
    <label v-if="node.op === 'matches'" class="field">
      正则（整串匹配）
      <input
        :value="node.pattern"
        :disabled="locked"
        @input="update('pattern', inputValue($event))"
      />
    </label>
    <label v-if="node.op === 'round' || node.op === 'floor' || node.op === 'ceil'" class="field">
      保留小数位
      <input
        type="number"
        min="0"
        max="6"
        step="1"
        :value="node.digits"
        :disabled="locked"
        @input="update('digits', digitsValue($event))"
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
        <option v-for="(label, key) in comparisonLabels" :key="key" :value="key">
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
        <!-- 根节点也占用引擎的 256 节点预算，因此子条件最多 255 条。 -->
        <button
          type="button"
          class="button"
          :disabled="locked || list.length >= 255"
          @click="update('expressions', [...list, { op: 'boolean', value: true }])"
        >
          添加条件
        </button>
      </template>
    </template>
    <p v-else-if="children.length || list.length" class="hint">已到最大嵌套层数，请简化表达式。</p>
  </div>
</template>
