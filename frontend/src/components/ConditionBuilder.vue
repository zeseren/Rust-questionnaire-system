<!-- 常用条件使用题目和选项名称，切换高级编辑不会重建或丢弃原表达式。 -->
<script setup lang="ts">
import { computed, ref, useId } from 'vue'
import type { ComputedVariable, Question } from '../types'
import {
  changeConditionOperator,
  conditionOperators,
  groupExpression,
  readConditionGroup,
} from '../condition-builder'
import type { ConditionGroup, ConditionOperator, SimpleCondition } from '../condition-builder'
import ExpressionEditor from './ExpressionEditor.vue'
import SearchSelect from './SearchSelect.vue'
import Icon from './Icon.vue'
const expression = defineModel<unknown>({ required: true })
const props = defineProps<{ questions: Question[]; variables: ComputedVariable[] }>()
const advanced = ref(false)
const id = useId()
const group = computed(() => readConditionGroup(expression.value))
const questionOptions = computed(() =>
  props.questions.map((question, index) => ({
    value: question.id,
    label: `${index + 1}. ${question.title || '未命名题目'}`,
  })),
)
const questionOf = (condition: SimpleCondition) =>
  props.questions.find((question) => question.id === condition.questionId)
function optionChoices(condition: SimpleCondition) {
  const question = questionOf(condition)
  return question && 'options' in question
    ? question.options.map((option) => ({ value: option.id, label: option.label || '未命名选项' }))
    : []
}
function defaultCondition(): SimpleCondition {
  return { questionId: props.questions[0]?.id ?? '', operator: 'answered', value: '' }
}
function setMode(event: Event) {
  const mode = (event.target as HTMLSelectElement).value as ConditionGroup['mode']
  expression.value = groupExpression({
    mode,
    conditions: group.value?.conditions.length ? group.value.conditions : [defaultCondition()],
  })
}
function edit(index: number, change: Partial<SimpleCondition>) {
  if (!group.value) return
  expression.value = groupExpression({
    ...group.value,
    conditions: group.value.conditions.map((condition, i) =>
      i === index ? { ...condition, ...change } : condition,
    ),
  })
}
function operator(index: number, value: string) {
  const condition = group.value?.conditions[index]
  if (!condition) return
  edit(index, changeConditionOperator(condition, value as ConditionOperator, questionOf(condition)))
}
function add() {
  if (group.value)
    expression.value = groupExpression({
      ...group.value,
      conditions: [...group.value.conditions, defaultCondition()],
    })
}
function remove(index: number) {
  if (group.value)
    expression.value = groupExpression({
      ...group.value,
      conditions: group.value.conditions.filter((_, i) => i !== index),
    })
}
</script>
<template>
  <div class="condition-builder">
    <div class="actions between">
      <span class="hint">{{ !group ? '此条件使用变量或嵌套计算' : '按回答设置条件' }}</span>
      <button
        v-if="group"
        type="button"
        class="text-button accent"
        :aria-pressed="advanced"
        @click="advanced = !advanced"
      >
        {{ advanced ? '返回简洁条件' : '高级表达式' }}
      </button>
    </div>
    <ExpressionEditor
      v-if="advanced || !group"
      v-model="expression"
      :questions="questions"
      :variables="variables"
    />
    <template v-else>
      <label class="condition-mode">
        条件成立方式
        <select :value="group.mode" @change="setMode">
          <option value="always">始终成立</option>
          <option value="never">始终不成立</option>
          <option value="all" :disabled="!questions.length">满足以下全部条件</option>
          <option value="any" :disabled="!questions.length">满足以下任一条件</option>
        </select>
      </label>
      <template v-if="group.mode === 'all' || group.mode === 'any'">
        <div v-for="(condition, index) in group.conditions" :key="index" class="condition-row">
          <span class="condition-join">
            {{ index === 0 ? '当' : group.mode === 'all' ? '且' : '或' }}
          </span>
          <div class="condition-inputs">
            <SearchSelect
              :model-value="condition.questionId"
              :options="questionOptions"
              :label="`条件 ${index + 1} 的题目`"
              placeholder="搜索题目"
              @update:model-value="
                edit(index, { questionId: $event, operator: 'answered', value: '' })
              "
            />
            <label class="sr-only" :for="`operator-${id}-${index}`">
              条件 {{ index + 1 }} 的比较方式
            </label>
            <select
              :id="`operator-${id}-${index}`"
              :value="condition.operator"
              @change="operator(index, ($event.target as HTMLSelectElement).value)"
            >
              <option
                v-if="
                  !conditionOperators(questionOf(condition)).some(
                    (item) => item.value === condition.operator,
                  )
                "
                :value="condition.operator"
                disabled
              >
                比较方式不可用
              </option>
              <option
                v-for="item in conditionOperators(questionOf(condition))"
                :key="item.value"
                :value="item.value"
              >
                {{ item.label }}
              </option>
            </select>
            <SearchSelect
              v-if="condition.operator === 'selected' || condition.operator === 'not_selected'"
              :model-value="condition.value"
              :options="optionChoices(condition)"
              :label="`条件 ${index + 1} 的选项`"
              placeholder="选择选项"
              @update:model-value="edit(index, { value: $event })"
            />
            <input
              v-else-if="condition.operator !== 'answered' && condition.operator !== 'not_answered'"
              :value="condition.value"
              :aria-label="`条件 ${index + 1} 的比较值`"
              :inputmode="condition.valueType === 'number' ? 'decimal' : 'text'"
              :placeholder="condition.valueType === 'number' ? '输入数字' : '输入文本'"
              @input="edit(index, { value: ($event.target as HTMLInputElement).value })"
            />
          </div>
          <button
            type="button"
            class="icon-button"
            :disabled="group.conditions.length <= 1"
            :aria-label="`删除条件 ${index + 1}`"
            @click="remove(index)"
          >
            <Icon name="close" />
          </button>
        </div>
        <button
          type="button"
          class="text-button accent"
          :disabled="!questions.length || group.conditions.length >= 64"
          @click="add"
        >
          <Icon name="plus" />
          添加条件
        </button>
      </template>
      <p v-else class="hint">
        {{
          group.mode === 'always'
            ? '这条规则会直接生效。选择条件成立方式，可以按回答判断。'
            : '这条规则不会生效，可以随时调整条件。'
        }}
      </p>
    </template>
  </div>
</template>
