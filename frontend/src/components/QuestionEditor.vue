<!-- 题型配置按契约编辑，服务器负责草稿容量和发布规则的最终校验。 -->
<script setup lang="ts">
import { newQuestion, TEXT_ANSWER_LIMITS } from '../documents'
import { questionNames } from '../format'
import type { Question } from '../types'
import Icon from './Icon.vue'
const question = defineModel<Question>({ required: true })
defineProps<{
  index: number
  total: number
  disabled?: boolean
  error?: string
  hasRule?: boolean
  ruleDisabled?: boolean
}>()
defineEmits<{ move: [direction: number]; duplicate: []; remove: []; rule: [] }>()
function changeType(event: Event) {
  const type = (event.target as HTMLSelectElement).value as Question['type']
  const before = question.value
  const next = newQuestion(type)
  next.id = before.id
  next.title = before.title
  next.required = before.required
  if ('options' in before && 'options' in next) next.options = before.options
  question.value = next
}
function addOption() {
  if ('options' in question.value)
    question.value.options.push({ id: crypto.randomUUID(), label: '' })
}
function bound(name: 'min' | 'max', event: Event) {
  // 非 number 题型没有 min/max 字段，这里的判定同时是类型收窄，不能删。
  if (question.value.type !== 'number') return
  const value = (event.target as HTMLInputElement).value
  question.value[name] = value === '' ? null : Number(value)
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
            class="icon-button"
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
            v-model.number="question.min_choices"
            type="number"
            min="0"
            max="50"
            step="1"
            required
          />
        </label>
        <label class="field">
          最多选择
          <input
            v-model.number="question.max_choices"
            type="number"
            min="1"
            max="50"
            step="1"
            required
          />
        </label>
      </div>
      <label
        v-if="question.type === 'text' || question.type === 'textarea'"
        class="field compact-field"
      >
        字数上限
        <input
          v-model.number="question.max_length"
          type="number"
          min="1"
          :max="TEXT_ANSWER_LIMITS[question.type]"
          step="1"
          required
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
              :value="question.min ?? ''"
              placeholder="不限"
              @input="bound('min', $event)"
            />
          </label>
          <label class="field">
            最大值
            <input
              type="number"
              step="any"
              :value="question.max ?? ''"
              placeholder="不限"
              @input="bound('max', $event)"
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
              v-model.number="question.min"
              type="number"
              step="1"
              min="-2147483648"
              max="2147483647"
              required
            />
          </label>
          <label class="field">
            最高分
            <input
              v-model.number="question.max"
              type="number"
              step="1"
              min="-2147483648"
              max="2147483647"
              required
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
        <button
          type="button"
          class="text-button accent"
          :disabled="ruleDisabled"
          @click="$emit('rule')"
        >
          <Icon name="grid" />
          {{ hasRule ? '编辑规则' : '设置规则' }}
        </button>
      </div>
    </fieldset>
  </article>
</template>
