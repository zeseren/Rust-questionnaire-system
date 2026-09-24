<!-- 同一题型组件用于预览和填写，文本始终通过 Vue 转义展示。 -->
<script setup lang="ts">
import type { Question, SurveyDocument } from '../types'
import type { Inputs, InputValue } from '../answers'
const values = defineModel<Inputs>({ required: true })
withDefaults(
  defineProps<{ document: SurveyDocument; errors?: Record<string, string>; disabled?: boolean }>(),
  { errors: () => ({}) },
)
function textValue(question: Question) {
  return String(values.value[question.id] ?? '')
}
function update(question: Question, event: Event) {
  values.value[question.id] = (event.target as HTMLInputElement).value
}
function toggle(question: Question, value: string) {
  const current = values.value[question.id]
  const list = Array.isArray(current) ? [...current] : []
  values.value[question.id] = list.includes(value)
    ? list.filter((item) => item !== value)
    : [...list, value]
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
            v-model="values[question.id]"
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
              v-model="values[question.id]"
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
          :value="textValue(question)"
          @input="update(question, $event)"
        />
        <div class="rating-labels">
          <span>{{ question.min_label || `${question.min} 分` }}</span>
          <span>{{ question.max_label || `${question.max} 分` }}</span>
        </div>
      </template>
      <button
        v-if="!question.required && values[question.id] != null"
        type="button"
        class="text-button mt"
        @click="values[question.id] = null"
      >
        清除此题回答
      </button>
      <p v-if="errors[question.id]" :id="`error-${question.id}`" class="field-error" role="alert">
        {{ errors[question.id] }}
      </p>
    </fieldset>
  </section>
</template>
