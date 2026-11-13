<!-- [入口在哪] 编辑器「规则与跳题」工作区直接渲染可视化配置或文本 DSL。 -->
<!-- [用户在第 1 步后悔了怎么删/退] 顶部标题栏常驻低饱和「移除全部逻辑」幽灵按钮（悬停变红并二次确认），规则/变量行内各自带删除入口，不必滚到页尾。 -->
<!-- [主提交通道在哪] 工作台内不设主按钮；唯一主按钮是编辑器固定栏的「发布问卷」，本地「应用文本规则」仅用于提交文本缓冲。 -->
<!-- 可视化配置和文本共用统一模型，未应用的文本在工作区切换时继续保留。 -->
<script setup lang="ts">
import { computed, nextTick, ref } from 'vue'
import type { DocumentLogic, Question, QuestionRule } from '../types'
import { logicDiagnostic } from '../editor-logic'
import ExpressionEditor from './ExpressionEditor.vue'
import ConditionBuilder from './ConditionBuilder.vue'
import FlowEditor from './FlowEditor.vue'
import LogicTextEditor from './LogicTextEditor.vue'
import SearchSelect from './SearchSelect.vue'
import Icon from './Icon.vue'
import { newId } from '../uuid'
const logic = defineModel<DocumentLogic | null | undefined>({ required: true })
const props = defineProps<{ questions: Question[]; disabled?: boolean }>()
const emit = defineEmits<{ pending: [value: boolean] }>()
const textMode = ref(false)
const textPending = ref(false)
const textEditor = ref<InstanceType<typeof LogicTextEditor>>()
const section = ref<'rules' | 'variables' | 'flow'>('rules')
function pending(value: boolean) {
  textPending.value = value
  emit('pending', value)
}
const diagnostic = computed(() =>
  logic.value ? logicDiagnostic(logic.value, props.questions) : null,
)
const ruleFields = {
  visible_if: { title: '显示条件', description: '满足条件时才展示这道题。' },
  required_if: {
    title: '动态必填',
    description: '满足条件时需要回答；题目原有的必填设置仍然有效。',
  },
  validate: { title: '答案约束', description: '回答需要满足此条件，可引用本题的答案。' },
} as const
const types = {
  boolean: '布尔',
  number: '数字',
  text: '文本',
  choice: '单选值',
  choices: '多选值',
} as const
const availableQuestions = computed(() =>
  props.questions.filter(
    (question) => !logic.value?.rules.some((rule) => rule.question_id === question.id),
  ),
)
const configuredRuleIds = computed(
  () => new Set(logic.value?.rules.map((rule) => rule.question_id) ?? []),
)
const ruleFieldKeys = ['visible_if', 'required_if', 'validate'] as const
function enable(): DocumentLogic {
  const created: DocumentLogic = {
    dsl_version: 1,
    semantics_version: 1,
    variables: [],
    rules: [],
  }
  logic.value = created
  return created
}
function disable() {
  if (props.disabled) return
  if (window.confirm('确定移除全部规则、变量和流程，恢复普通问卷吗？')) {
    logic.value = null
    textMode.value = false
  }
}
async function addRule(questionId = availableQuestions.value[0]?.id) {
  if (!questionId || props.disabled) return
  const current = logic.value ?? enable()
  const existing = current.rules.find((rule) => rule.question_id === questionId)
  if (!existing)
    current.rules.push({ question_id: questionId, visible_if: { op: 'boolean', value: true } })
  textMode.value = false
  section.value = 'rules'
  await nextTick()
  const index = current.rules.findIndex((rule) => rule.question_id === questionId)
  document.getElementById(`logic-rule-${index}`)?.focus()
}
function addVariable() {
  if (props.disabled) return
  const current = logic.value ?? enable()
  current.variables.push({
    id: newId(),
    value_type: 'boolean',
    expression: { op: 'boolean', value: true },
  })
}
function toggleRule(rule: QuestionRule, key: keyof typeof ruleFields) {
  if (rule[key] != null) {
    delete rule[key]
    if (!ruleFieldKeys.some((field) => rule[field] != null))
      logic.value?.rules.splice(logic.value.rules.indexOf(rule), 1)
  } else rule[key] = { op: 'boolean', value: true }
}
function references(rule: QuestionRule, field: keyof typeof ruleFields) {
  const target = props.questions.findIndex((question) => question.id === rule.question_id)
  return props.questions.slice(0, Math.max(0, target + (field === 'validate' ? 1 : 0)))
}
async function locate() {
  if (!diagnostic.value) return
  const target = diagnostic.value.target
  textMode.value = false
  section.value = target.startsWith('logic-variable')
    ? 'variables'
    : target.startsWith('logic-node') || target === 'logic-membership'
      ? 'flow'
      : 'rules'
  await nextTick()
  document.getElementById(target)?.focus()
}
/** 切到文本模式：没有未应用草稿时按当前可视化规则重新生成，避免文本过期。 */
function openText() {
  textMode.value = true
  if (!textPending.value) textEditor.value?.discard()
}
defineExpose({ addRule, locate, applyText, discardText })
/** 保存前应用文本草稿；失败时编辑器已聚焦到错误位置。 */
function applyText(): boolean {
  return textEditor.value?.apply() ?? false
}
/** 保存前忽略文本草稿：重新按当前可视化规则生成文本。 */
function discardText() {
  textEditor.value?.discard()
}
</script>
<template>
  <section id="logic-editor" class="logic-workbench" tabindex="-1">
    <div class="logic-workbench-heading">
      <div>
        <p class="eyebrow">让问题恰到好处</p>
        <h2>规则与跳题</h2>
        <p class="hint">根据回答显示问题、计算结果，并安排填写路径。</p>
      </div>
      <div v-if="logic" class="actions">
        <button
          type="button"
          class="text-button danger-text"
          :disabled="disabled || textPending"
          @click="disable"
        >
          移除全部逻辑
        </button>
        <div class="view-control" role="group" aria-label="规则编辑方式">
          <button type="button" class="button" :aria-pressed="!textMode" @click="textMode = false">
            可视化配置
          </button>
          <button type="button" class="button" :aria-pressed="textMode" @click="openText">
            文本 DSL
          </button>
        </div>
      </div>
    </div>
    <div v-if="!logic" class="logic-intro panel">
      <div class="logic-features">
        <div>
          <span class="template-icon"><Icon name="document" /></span>
          <h3>按需提问</h3>
          <p class="hint">根据前面的回答设置显示、必填与校验。</p>
        </div>
        <div>
          <span class="template-icon violet"><Icon name="chart" /></span>
          <h3>计算与判断</h3>
          <p class="hint">将常用判断定义为变量，在规则中复用。</p>
        </div>
        <div>
          <span class="template-icon teal"><Icon name="grid" /></span>
          <h3>自然地分流</h3>
          <p class="hint">用分页与条件分支组织不同的填写路径。</p>
        </div>
      </div>
      <button type="button" class="button" :disabled="disabled" @click="enable">
        <Icon name="plus" />
        开始配置规则
      </button>
    </div>
    <template v-else>
      <p v-if="textPending && !textMode" class="notice">
        文本 DSL 有尚未应用的修改。切回「文本 DSL」可应用或恢复；保存时会询问如何处理这段草稿。
      </p>
      <div v-show="textMode">
        <LogicTextEditor
          v-if="logic"
          ref="textEditor"
          v-model="logic"
          :questions="questions"
          :disabled="disabled"
          @pending="pending"
        />
      </div>
      <div v-show="!textMode" class="panel logic-visual">
        <div class="logic-status" :class="{ 'has-issue': diagnostic }" role="status">
          <span class="status-dot"></span>
          <div class="grow">
            <strong>{{ diagnostic ? diagnostic.message : '规则检查通过' }}</strong>
            <p v-if="diagnostic" class="hint">{{ diagnostic.path }}</p>
            <p v-else class="hint">可随时试填，检查规则实际生效的结果。</p>
          </div>
          <button v-if="diagnostic" type="button" class="button" @click="locate">定位配置</button>
        </div>
        <div class="logic-tabs" role="group" aria-label="可视化配置分类">
          <button type="button" :aria-pressed="section === 'rules'" @click="section = 'rules'">
            题目规则
            <span>{{ logic.rules.length }}</span>
          </button>
          <button
            type="button"
            :aria-pressed="section === 'variables'"
            @click="section = 'variables'"
          >
            计算变量
            <span>{{ logic.variables.length }}</span>
          </button>
          <button type="button" :aria-pressed="section === 'flow'" @click="section = 'flow'">
            页面流程
            <span>{{ logic.flow?.nodes.length ?? 0 }}</span>
          </button>
        </div>
        <fieldset :disabled="disabled" style="border: 0; padding: 0; margin: 0; min-width: 0">
          <div v-if="section === 'rules'" class="form-stack">
            <div class="actions between">
              <div>
                <h3>题目规则</h3>
                <p class="hint">先选择一道题，再设置它在什么情况下显示或必填。</p>
              </div>
              <button
                type="button"
                class="button"
                :disabled="!availableQuestions.length"
                @click="addRule()"
              >
                <Icon name="plus" />
                添加规则
              </button>
            </div>
            <div v-if="!logic.rules.length" class="logic-empty">
              <Icon name="document" />
              <h3>每位填写者，只回答相关的问题</h3>
              <p class="hint">例如：选中“其他”后显示补充说明，或在评分较低时追问原因。</p>
              <button type="button" class="button" :disabled="!questions.length" @click="addRule()">
                创建第一条规则
              </button>
            </div>
            <section
              v-for="(rule, index) in logic.rules"
              :id="`logic-rule-${index}`"
              :key="rule.question_id || index"
              tabindex="-1"
              class="logic-block rule-card"
            >
              <div class="rule-card-heading">
                <span class="rule-number">{{ String(index + 1).padStart(2, '0') }}</span>
                <div class="grow">
                  <label class="field">
                    作用题目
                    <SearchSelect
                      v-model="rule.question_id"
                      :label="`规则 ${index + 1} 的作用题目`"
                      :options="
                        questions.map((question, i) => ({
                          value: question.id,
                          label: `${i + 1}. ${question.title || '未命名题目'}`,
                          disabled:
                            configuredRuleIds.has(question.id) && question.id !== rule.question_id,
                        }))
                      "
                    />
                  </label>
                </div>
                <button
                  type="button"
                  class="icon-button danger-text"
                  :aria-label="`删除规则 ${index + 1}`"
                  @click="logic.rules.splice(index, 1)"
                >
                  <Icon name="trash" />
                </button>
              </div>
              <div
                v-for="(details, field) in ruleFields"
                :key="field"
                class="rule-field"
                :class="{ enabled: rule[field] != null }"
              >
                <label class="rule-toggle">
                  <input
                    type="checkbox"
                    :checked="rule[field] != null"
                    @change="toggleRule(rule, field)"
                  />
                  <span>
                    <strong>{{ details.title }}</strong>
                    <span class="hint">{{ details.description }}</span>
                  </span>
                </label>
                <ConditionBuilder
                  v-if="rule[field] != null"
                  v-model="rule[field]"
                  :questions="references(rule, field)"
                  :variables="logic.variables"
                />
              </div>
            </section>
          </div>
          <div v-else-if="section === 'variables'" class="form-stack">
            <div class="actions between">
              <div>
                <h3>计算变量</h3>
                <p class="hint">复用评分或判断结果。变量会按依赖顺序自动计算。</p>
              </div>
              <button
                type="button"
                class="button"
                :disabled="disabled || logic.variables.length >= 100"
                @click="addVariable"
              >
                <Icon name="plus" />
                添加变量
              </button>
            </div>
            <div v-if="!logic.variables.length" class="logic-empty">
              <Icon name="chart" />
              <h3>把常用计算只写一次</h3>
              <p class="hint">例如：计算总分，或判断是否需要后续回访。</p>
            </div>
            <section
              v-for="(variable, index) in logic.variables"
              :id="`logic-variable-${variable.id}`"
              :key="variable.id"
              tabindex="-1"
              class="logic-block form-stack"
            >
              <div class="actions between">
                <strong>
                  变量 {{ index + 1 }}
                  <code class="hint">v{{ index + 1 }}</code>
                </strong>
                <button
                  type="button"
                  class="text-button danger-text"
                  @click="logic.variables.splice(index, 1)"
                >
                  删除变量
                </button>
              </div>
              <label class="field">
                值类型
                <select v-model="variable.value_type">
                  <option v-for="(label, type) in types" :key="type" :value="type">
                    {{ label }}
                  </option>
                </select>
              </label>
              <ExpressionEditor
                v-model="variable.expression"
                :questions="questions"
                :variables="logic.variables"
              />
            </section>
          </div>
          <FlowEditor
            v-else
            v-model="logic.flow"
            :questions="questions"
            :variables="logic.variables"
          />
        </fieldset>
      </div>
      <div class="logic-footer">
        <span class="hint">
          {{ logic.rules.length }} 条题目规则 · {{ logic.variables.length }} 个变量 ·
          {{
            logic.flow
              ? `${logic.flow.nodes.filter((node) => node.type === 'page').length} 个页面`
              : '单页填写'
          }}
        </span>
        <span class="hint">移除全部逻辑可在上方标题栏随时操作。</span>
      </div>
    </template>
  </section>
</template>
