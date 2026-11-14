<!-- [入口在哪] 编辑器「题目设计」工作区逐题渲染，也可从右侧「题目大纲」定位到指定题目卡片。 -->
<!-- [用户在第 1 步后悔了怎么删/退] 每张题目卡片工具栏最右侧常驻「删除题目」幽灵按钮，悬停变红；删除前由编辑器父级弹窗确认，误删可放弃修改。 -->
<!-- [主提交通道在哪] 题目卡片内不设主按钮；保存与发布统一由编辑器顶部固定栏承担，卡片内「设置规则」为次级操作。 -->
<!-- 题型配置按契约编辑，服务器负责草稿容量和发布规则的最终校验。 -->
<script setup lang="ts">
import { ref } from 'vue'
import { newQuestion, TEXT_ANSWER_LIMITS } from '../documents'
import { questionNames } from '../format'
import { newId } from '../uuid'
import type { Choice, Question } from '../types'
import Icon from './Icon.vue'
const question = defineModel<Question>({ required: true })
defineProps<{
  index: number
  total: number
  disabled?: boolean
  error?: string
  hasRule?: boolean
}>()
defineEmits<{ move: [direction: number]; duplicate: []; remove: []; rule: [] }>()
/** 数字输入的本地草稿：允许暂时为空或只有负号，不立即回写模型。 */
const drafts = ref<Record<string, string>>({})
/** 最近一次单选/多选选项，切到文本或数字题后仍可在切回时恢复。 */
const lastOptions = ref<Choice[]>([])
function isQuestionType(value: string): value is Question['type'] {
  return Object.hasOwn(questionNames, value)
}
function changeType(event: Event) {
  if (!(event.currentTarget instanceof HTMLSelectElement)) return
  const value = event.currentTarget.value
  if (!isQuestionType(value)) return
  const before = question.value
  const next = newQuestion(value)
  next.id = before.id
  next.title = before.title
  next.required = before.required
  // 只有当前题型确实带选项时才覆盖缓存，避免空列表冲掉已有选项。
  if ('options' in before && before.options.length)
    lastOptions.value = before.options.map((option) => ({ ...option }))
  if ('options' in next && lastOptions.value.length)
    next.options = lastOptions.value.map((option) => ({ ...option }))
  drafts.value = {}
  question.value = next
}
function addOption() {
  if ('options' in question.value) question.value.options.push({ id: newId(), label: '' })
}

type IntegerField = 'min_choices' | 'max_choices' | 'max_length' | 'rating_min' | 'rating_max'
type NumberBoundField = 'number_min' | 'number_max'
type NumericField = IntegerField | NumberBoundField

type NumericDescriptor =
  | { allowEmpty: false; read: number; write: (value: number) => void }
  | { allowEmpty: true; read: number | null; write: (value: number | null) => void }

/** 按字段定位当前题目的数值配置；题型不匹配时返回 null。 */
function numericField(field: NumericField): NumericDescriptor | null {
  const current = question.value
  switch (field) {
    case 'min_choices':
      return current.type === 'multiple_choice'
        ? { allowEmpty: false, read: current.min_choices, write: (v) => (current.min_choices = v) }
        : null
    case 'max_choices':
      return current.type === 'multiple_choice'
        ? { allowEmpty: false, read: current.max_choices, write: (v) => (current.max_choices = v) }
        : null
    case 'max_length':
      return current.type === 'text' || current.type === 'textarea'
        ? { allowEmpty: false, read: current.max_length, write: (v) => (current.max_length = v) }
        : null
    case 'rating_min':
      return current.type === 'rating'
        ? { allowEmpty: false, read: current.min, write: (v) => (current.min = v) }
        : null
    case 'rating_max':
      return current.type === 'rating'
        ? { allowEmpty: false, read: current.max, write: (v) => (current.max = v) }
        : null
    case 'number_min':
      return current.type === 'number'
        ? { allowEmpty: true, read: current.min, write: (v) => (current.min = v) }
        : null
    case 'number_max':
      return current.type === 'number'
        ? { allowEmpty: true, read: current.max, write: (v) => (current.max = v) }
        : null
  }
}

/** 输入中展示草稿原文，未编辑时展示模型值；空上下限渲染为空串以保留占位符。 */
function draftText(field: NumericField, current: number | null): string {
  return drafts.value[field] ?? (current === null ? '' : String(current))
}

/** 输入过程中仅在合法数值时回写；空串与过渡正负号留在草稿里。 */
function inputNumeric(field: NumericField, event: Event) {
  const target = numericField(field)
  if (!target || !(event.currentTarget instanceof HTMLInputElement)) return
  const raw = event.currentTarget.value
  drafts.value[field] = raw
  const text = raw.trim()
  if (text === '' || text === '-' || text === '+') return
  const parsed = Number(text)
  if (target.allowEmpty) {
    if (Number.isFinite(parsed)) target.write(parsed)
  } else if (Number.isInteger(parsed)) {
    target.write(parsed)
  }
}

/** 失焦时结算草稿：可选上下限空串回写 null，非法过渡文本恢复模型值。 */
function commitNumeric(field: NumericField, event: Event) {
  const target = numericField(field)
  if (!target || !(event.currentTarget instanceof HTMLInputElement)) return
  const input = event.currentTarget
  const raw = (drafts.value[field] ?? input.value).trim()
  delete drafts.value[field]
  if (raw === '' && target.allowEmpty) {
    target.write(null)
    input.value = ''
    return
  }
  const parsed = Number(raw)
  const valid =
    raw !== '' &&
    raw !== '-' &&
    raw !== '+' &&
    (target.allowEmpty ? Number.isFinite(parsed) : Number.isInteger(parsed))
  if (valid) {
    target.write(parsed)
    input.value = String(parsed)
    return
  }
  input.value = target.read === null ? '' : String(target.read)
}
</script>
<template>
  <article
    :id="`question-${question.id}`"
    class="panel question-editor"
    :class="{ 'panel-error': error }"
    tabindex="-1"
  >
    <fieldset :disabled="disabled">
      <div class="question-toolbar">
        <span class="question-number">{{ String(index + 1).padStart(2, '0') }}</span>
        <label class="sr-only" :for="`type-${question.id}`">第 {{ index + 1 }} 题题型</label>
        <select :id="`type-${question.id}`" :value="question.type" @change="changeType">
          <option v-for="(label, type) in questionNames" :key="type" :value="type">
            {{ label }}
          </option>
        </select>
        <div class="grow"></div>
        <label class="check-label">
          <input v-model="question.required" type="checkbox" />
          必填
        </label>
        <button
          type="button"
          class="icon-button"
          :disabled="index === 0"
          :aria-label="`上移第 ${index + 1} 题`"
          @click="$emit('move', -1)"
        >
          <Icon name="up" />
        </button>
        <button
          type="button"
          class="icon-button"
          :disabled="index === total - 1"
          :aria-label="`下移第 ${index + 1} 题`"
          @click="$emit('move', 1)"
        >
          <Icon name="down" />
        </button>
        <button
          type="button"
          class="icon-button"
          :disabled="total >= 100"
          :aria-label="`复制第 ${index + 1} 题`"
          @click="$emit('duplicate')"
        >
          <Icon name="copy" />
        </button>
        <button
          type="button"
          class="icon-button danger-text"
          style="margin-left: 8px"
          :aria-label="`删除第 ${index + 1} 题`"
          @click="$emit('remove')"
        >
          <Icon name="trash" />
        </button>
      </div>
      <label class="field">
        题目标题
        <input v-model="question.title" placeholder="输入你想问的问题" />
      </label>
      <p v-if="error" class="field-error" role="alert">{{ error }}</p>
      <div v-if="'options' in question" class="option-editor">
        <div v-for="(option, optionIndex) in question.options" :key="option.id" class="option-row">
          <span
            class="option-marker"
            :class="{ square: question.type === 'multiple_choice' }"
          ></span>
          <label class="sr-only" :for="option.id">选项 {{ optionIndex + 1 }}</label>
          <input :id="option.id" v-model="option.label" :placeholder="`选项 ${optionIndex + 1}`" />
          <button
            type="button"
            class="icon-button danger-text"
            :aria-label="`删除选项 ${optionIndex + 1}`"
            @click="question.options.splice(optionIndex, 1)"
          >
            <Icon name="close" />
          </button>
        </div>
        <button
          type="button"
          class="text-button accent"
          :disabled="question.options.length >= 50"
          @click="addOption"
        >
          <Icon name="plus" />
          添加选项
        </button>
      </div>
      <div v-if="question.type === 'multiple_choice'" class="field-row">
        <label class="field">
          最少选择
          <input
            :value="draftText('min_choices', question.min_choices)"
            type="number"
            min="0"
            max="50"
            step="1"
            required
            @input="inputNumeric('min_choices', $event)"
            @blur="commitNumeric('min_choices', $event)"
          />
        </label>
        <label class="field">
          最多选择
          <input
            :value="draftText('max_choices', question.max_choices)"
            type="number"
            min="1"
            max="50"
            step="1"
            required
            @input="inputNumeric('max_choices', $event)"
            @blur="commitNumeric('max_choices', $event)"
          />
        </label>
      </div>
      <label
        v-if="question.type === 'text' || question.type === 'textarea'"
        class="field compact-field"
      >
        字数上限
        <input
          :value="draftText('max_length', question.max_length)"
          type="number"
          min="1"
          :max="TEXT_ANSWER_LIMITS[question.type]"
          step="1"
          required
          @input="inputNumeric('max_length', $event)"
          @blur="commitNumeric('max_length', $event)"
        />
        <span class="hint">最多可设置为 {{ TEXT_ANSWER_LIMITS[question.type] }} 个字符。</span>
      </label>
      <template v-if="question.type === 'number'">
        <div class="field-row">
          <label class="field">
            最小值
            <input
              type="number"
              step="any"
              :value="draftText('number_min', question.min)"
              placeholder="不限"
              @input="inputNumeric('number_min', $event)"
              @blur="commitNumeric('number_min', $event)"
            />
          </label>
          <label class="field">
            最大值
            <input
              type="number"
              step="any"
              :value="draftText('number_max', question.max)"
              placeholder="不限"
              @input="inputNumeric('number_max', $event)"
              @blur="commitNumeric('number_max', $event)"
            />
          </label>
        </div>
        <label class="check-label">
          <input v-model="question.integer" type="checkbox" />
          只接受整数
        </label>
      </template>
      <template v-if="question.type === 'rating'">
        <div class="field-row">
          <label class="field">
            最低分
            <input
              :value="draftText('rating_min', question.min)"
              type="number"
              step="1"
              min="-2147483648"
              max="2147483647"
              required
              @input="inputNumeric('rating_min', $event)"
              @blur="commitNumeric('rating_min', $event)"
            />
          </label>
          <label class="field">
            最高分
            <input
              :value="draftText('rating_max', question.max)"
              type="number"
              step="1"
              min="-2147483648"
              max="2147483647"
              required
              @input="inputNumeric('rating_max', $event)"
              @blur="commitNumeric('rating_max', $event)"
            />
          </label>
        </div>
        <div class="field-row">
          <label class="field">
            最低分说明
            <input v-model="question.min_label" placeholder="可选" />
          </label>
          <label class="field">
            最高分说明
            <input v-model="question.max_label" placeholder="可选" />
          </label>
        </div>
      </template>
      <div class="question-logic-footer">
        <span class="hint">
          {{ hasRule ? '此题已配置条件规则' : '可以按前面的回答设置显示与必填' }}
        </span>
        <button type="button" class="text-button accent" @click="$emit('rule')">
          <Icon name="grid" />
          {{ hasRule ? '编辑规则' : '设置规则' }}
        </button>
      </div>
    </fieldset>
  </article>
</template>
