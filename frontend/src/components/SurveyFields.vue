<!-- [入口在哪] 填写页/预览页逐题渲染的题面卡片，每题自带「清除此题回答」。 -->
<!-- [用户在第 1 步后悔了怎么删/退] 非必填题可就地清除本题回答；整卷退出与重置由外层固定栏负责。 -->
<!-- [主提交通道在哪] 本组件不提交，唯一主提交在页面底部固定栏右侧。 -->
<!-- 同一题型组件用于预览和填写，文本始终通过 Vue 转义展示。 -->
<script setup lang="ts">
import type { Question, SurveyDocument } from '../types'
import type { Inputs, InputValue } from '../answers'
const values = defineModel<Inputs>({ required: true })
const emit = defineEmits<{ change: [id: string, value: InputValue]; commit: [id: string] }>()
withDefaults(
  defineProps<{ document: SurveyDocument; errors?: Record<string, string>; disabled?: boolean }>(),
  { errors: () => ({}) },
)
function textValue(question: Question) {
  return String(values.value[question.id] ?? '')
}
function update(question: Question, event: Event) {
  const target = event.currentTarget
  if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement)
    change(question, target.value)
}
function change(question: Question, value: InputValue) {
  values.value = { ...values.value, [question.id]: value }
  emit('change', question.id, value)
}
function toggle(question: Question, value: string) {
  const current = values.value[question.id]
  const list = Array.isArray(current) ? [...current] : []
  change(question, list.includes(value) ? list.filter((item) => item !== value) : [...list, value])
}
function selected(value: InputValue | undefined, id: string) {
  return Array.isArray(value) && value.includes(id)
}
function scores(question: Question) {
  return question.type === 'rating' &&
    question.max >= question.min &&
    question.max - question.min <= 10
    ? Array.from({ length: question.max - question.min + 1 }, (_, index) => question.min + index)
    : []
}
</script>
<template>
  <section
    v-for="(question, index) in document.questions"
    :id="`answer-${question.id}`"
    :key="question.id"
    class="panel survey-question"
    :class="{ 'panel-error': errors[question.id] }"
    tabindex="-1"
  >
    <fieldset
      :disabled="disabled"
      :aria-describedby="errors[question.id] ? `error-${question.id}` : undefined"
    >
      <legend :id="`label-${question.id}`" class="question-legend">
        <span>{{ String(index + 1).padStart(2, '0') }}</span>
        <span class="grow">{{ question.title || '未填写题目标题' }}</span>
        <span class="required">{{ question.required ? '必填' : '选填' }}</span>
      </legend>
      <div v-if="question.type === 'single_choice'" class="answer-options">
        <label v-for="option in question.options" :key="option.id" class="answer-option">
          <input
            :checked="values[question.id] === option.id"
            @change="change(question, option.id)"
            type="radio"
            :name="question.id"
            :value="option.id"
          />
          <span>{{ option.label || '未填写选项' }}</span>
        </label>
      </div>
      <template v-else-if="question.type === 'multiple_choice'">
        <p class="hint mb">选择 {{ question.min_choices }}–{{ question.max_choices }} 项</p>
        <div class="answer-options">
          <label v-for="option in question.options" :key="option.id" class="answer-option">
            <input
              type="checkbox"
              :checked="selected(values[question.id], option.id)"
              @change="toggle(question, option.id)"
            />
            <span>{{ option.label || '未填写选项' }}</span>
          </label>
        </div>
      </template>
      <template v-else-if="question.type === 'text' || question.type === 'textarea'">
        <textarea
          v-if="question.type === 'textarea'"
          class="answer-input"
          :aria-labelledby="`label-${question.id}`"
          :aria-invalid="!!errors[question.id]"
          :value="textValue(question)"
          placeholder="写下你的想法…"
          @input="update(question, $event)"
        />
        <input
          v-else
          class="answer-input"
          type="text"
          :aria-labelledby="`label-${question.id}`"
          :aria-invalid="!!errors[question.id]"
          :value="textValue(question)"
          placeholder="请输入回答"
          @input="update(question, $event)"
        />
        <p class="hint mt">
          {{ [...textValue(question)].length }} / {{ question.max_length }} 字符
        </p>
      </template>
      <template v-else-if="question.type === 'number'">
        <input
          class="answer-input"
          type="text"
          inputmode="decimal"
          :aria-labelledby="`label-${question.id}`"
          :aria-invalid="!!errors[question.id]"
          :value="textValue(question)"
          placeholder="请输入数字"
          @input="update(question, $event)"
          @blur="emit('commit', question.id)"
        />
        <p class="hint mt">
          {{ question.integer ? '仅限整数' : '支持整数和小数'
          }}{{ question.min != null ? `，最小 ${question.min}` : ''
          }}{{ question.max != null ? `，最大 ${question.max}` : '' }}
        </p>
      </template>
      <template v-else-if="question.type === 'rating'">
        <div v-if="scores(question).length" class="rating-options">
          <label v-for="score in scores(question)" :key="score" class="rating-option">
            <input
              :checked="values[question.id] === score"
              @change="change(question, score)"
              type="radio"
              :name="question.id"
              :value="score"
              :aria-label="`${score} 分`"
            />
            <span>{{ score }}</span>
          </label>
        </div>
        <input
          v-else
          class="answer-input"
          type="number"
          step="1"
          :min="question.min"
          :max="question.max"
          :aria-labelledby="`label-${question.id}`"
          :aria-invalid="!!errors[question.id]"
          :value="textValue(question)"
          @input="update(question, $event)"
          @blur="emit('commit', question.id)"
        />
        <div class="rating-labels">
          <span>{{ question.min_label || `${question.min} 分` }}</span>
          <span>{{ question.max_label || `${question.max} 分` }}</span>
        </div>
      </template>
      <button
        v-if="!question.required && values[question.id] != null"
        type="button"
        class="text-button danger-text mt"
        @click="change(question, null)"
      >
        清除此题回答
      </button>
      <p v-if="errors[question.id]" :id="`error-${question.id}`" class="field-error" role="alert">
        {{ errors[question.id] }}
      </p>
    </fieldset>
  </section>
</template>
