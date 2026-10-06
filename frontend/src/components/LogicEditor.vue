<!-- 可视化配置直接写入统一规则模型，删除不会静默修复依赖引用。 -->
<script setup lang="ts">
import { computed, ref } from 'vue'
import type { Question } from '../types'
import type { DocumentLogic, QuestionRule } from '../document-logic'
import type { FlowNode } from '../flow-compiler'
import { logicDiagnostic } from '../editor-logic'
import ExpressionEditor from './ExpressionEditor.vue'
import FlowTarget from './FlowTarget.vue'
import LogicTextEditor from './LogicTextEditor.vue'
const logic = defineModel<DocumentLogic | null | undefined>({ required: true })
const props = defineProps<{ questions: Question[] }>()
const emit = defineEmits<{ pending: [value: boolean] }>()
const textMode = ref(false),
  textPending = ref(false)
function pending(value: boolean) {
  textPending.value = value
  emit('pending', value)
}
const diagnostic = computed(() =>
  logic.value ? logicDiagnostic(logic.value, props.questions) : null,
)
const ruleFields = {
  visible_if: '显示条件',
  required_if: '动态必填',
  validate: '答案约束',
} as const
const types = {
  boolean: '布尔',
  number: '数字',
  text: '文本',
  choice: '单选值',
  choices: '多选值',
} as const
function addVariable() {
  logic.value?.variables.push({
    id: crypto.randomUUID(),
    value_type: 'boolean',
    expression: { op: 'boolean', value: true },
  })
}
function enable() {
  logic.value = { dsl_version: 1, semantics_version: 1, variables: [], rules: [] }
}
function disable() {
  if (window.confirm('确定移除全部规则、变量和流程，恢复普通问卷吗？')) logic.value = null
}
function startFlow() {
  if (!logic.value || !props.questions.length) return
  const entry = crypto.randomUUID(),
    end = crypto.randomUUID()
  logic.value.flow = {
    entry,
    nodes: [
      { type: 'page', id: entry, question_ids: props.questions.map((q) => q.id), next: end },
      { type: 'end', id: end, outcome: 'complete' },
    ],
  }
}
function addNode(index: number, type: FlowNode['type']) {
  const flow = logic.value?.flow
  if (!flow) return
  const id = crypto.randomUUID(),
    target = flow.nodes[index + 1]?.id ?? ''
  const node: FlowNode =
    type === 'page'
      ? { type, id, question_ids: [], next: target }
      : type === 'branch'
        ? {
            type,
            id,
            branches: [{ when: { op: 'boolean', value: true }, target }],
            otherwise: target,
          }
        : { type, id, outcome: 'complete' }
  const previous = flow.nodes[index]
  if (previous?.type === 'page' && previous.next === target) previous.next = id
  if (previous?.type === 'branch' && previous.otherwise === target) previous.otherwise = id
  flow.nodes.splice(index + 1, 0, node)
}
function moveNode(index: number, direction: number) {
  const flow = logic.value?.flow
  const to = index + direction
  if (!flow || to < 0 || to >= flow.nodes.length) return
  flow.nodes.splice(to, 0, flow.nodes.splice(index, 1)[0]!)
  // 入口身份不随顺序悄悄变化；编译器会提示错误入口及反向边。
}
function assign(questionId: string, pageId: string) {
  for (const node of logic.value?.flow?.nodes ?? [])
    if (node.type === 'page') {
      node.question_ids = node.question_ids.filter((id) => id !== questionId)
      if (node.id === pageId) node.question_ids.push(questionId)
      node.question_ids.sort(
        (a, b) =>
          props.questions.findIndex((q) => q.id === a) -
          props.questions.findIndex((q) => q.id === b),
      )
    }
}
function pageOf(id: string) {
  return (
    logic.value?.flow?.nodes.find((n) => n.type === 'page' && n.question_ids.includes(id))?.id ?? ''
  )
}
function toggleRule(rule: QuestionRule, key: keyof typeof ruleFields) {
  if (rule[key] != null) delete rule[key]
  else rule[key] = { op: 'boolean', value: true }
}
function locate() {
  if (diagnostic.value) document.getElementById(diagnostic.value.target)?.focus()
}
</script>
<template>
  <section id="logic-editor" class="panel form-stack mt" tabindex="-1">
    <div class="actions between">
      <h2>规则与跳题</h2>
      <button
        v-if="logic"
        type="button"
        class="text-button danger-text"
        :disabled="textPending"
        @click="disable"
      >
        移除全部逻辑
      </button>
    </div>
    <template v-if="!logic">
      <p class="hint">按回答设置显隐、必填与校验，也可以配置分页、分支和汇合。</p>
      <button type="button" class="button" @click="enable">启用规则与跳题</button>
    </template>
    <template v-else>
      <div class="actions">
        <button
          type="button"
          class="button"
          :disabled="textPending"
          :aria-pressed="!textMode"
          @click="textMode = false"
        >
          可视化配置
        </button>
        <button type="button" class="button" :aria-pressed="textMode" @click="textMode = true">
          文本 DSL
        </button>
      </div>
      <LogicTextEditor v-if="textMode" v-model="logic" :questions="questions" @pending="pending" />
      <template v-else>
        <div v-if="diagnostic" class="notice" role="status">
          <div class="grow">
            {{ diagnostic.message }}
            <p class="hint">{{ diagnostic.path }}</p>
          </div>
          <button type="button" class="button" @click="locate">定位配置</button>
        </div>
        <p v-else class="hint">规则检查通过。修改或删除题目后，请再次检查依赖。</p>
        <h3>计算变量</h3>
        <section
          v-for="(variable, index) in logic.variables"
          :id="`logic-variable-${variable.id}`"
          :key="variable.id"
          tabindex="-1"
          class="logic-block form-stack"
        >
          <div class="actions between">
            <strong>变量 {{ index + 1 }}</strong>
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
              <option v-for="(label, type) in types" :key="type" :value="type">{{ label }}</option>
            </select>
          </label>
          <ExpressionEditor
            v-model="variable.expression"
            :questions="questions"
            :variables="logic.variables"
          />
        </section>
        <button
          type="button"
          class="button"
          :disabled="logic.variables.length >= 100"
          @click="addVariable"
        >
          添加计算变量
        </button>
        <h3>题目规则</h3>
        <p class="hint">显示和必填条件只能读取前面的题目；约束还可读取本题。静态必填始终有效。</p>
        <section
          v-for="(rule, index) in logic.rules"
          :id="`logic-rule-${index}`"
          :key="index"
          tabindex="-1"
          class="logic-block form-stack"
        >
          <div class="actions between">
            <strong>规则 {{ index + 1 }}</strong>
            <button
              type="button"
              class="text-button danger-text"
              @click="logic.rules.splice(index, 1)"
            >
              删除规则
            </button>
          </div>
          <label class="field">
            作用题目
            <select v-model="rule.question_id">
              <option
                v-if="!questions.some((q) => q.id === rule.question_id)"
                :value="rule.question_id"
              >
                请选择题目（当前引用不存在）
              </option>
              <option v-for="(q, i) in questions" :key="q.id" :value="q.id">
                {{ i + 1 }}. {{ q.title || '未命名题目' }}
              </option>
            </select>
          </label>
          <div v-for="(label, field) in ruleFields" :key="field" class="form-stack">
            <label>
              <input
                type="checkbox"
                :checked="rule[field] != null"
                @change="toggleRule(rule, field)"
              />
              {{ label }}
            </label>
            <ExpressionEditor
              v-if="rule[field] != null"
              v-model="rule[field]"
              :questions="questions"
              :variables="logic.variables"
            />
          </div>
        </section>
        <button
          type="button"
          class="button"
          :disabled="logic.rules.length >= 100 || !questions.length"
          @click="
            logic.rules.push({
              question_id:
                questions.find((q) => !logic?.rules.some((r) => r.question_id === q.id))?.id ?? '',
              visible_if: { op: 'boolean', value: true },
            })
          "
        >
          添加题目规则
        </button>
        <h3>页面流程</h3>
        <template v-if="!logic.flow">
          <p class="hint">当前在同一页填写全部可见题目。</p>
          <button type="button" class="button" :disabled="!questions.length" @click="startFlow">
            配置分页与分支
          </button>
        </template>
        <template v-else>
          <p class="hint">
            节点按顺序排列，只能跳到后续节点。分支从上到下匹配，第一个成立的条件生效。修改出口即可让多条路径汇合。
          </p>
          <label class="field">
            入口页面
            <select v-model="logic.flow.entry">
              <option
                v-for="(node, i) in logic.flow.nodes"
                :key="node.id"
                :value="node.id"
                :disabled="node.type !== 'page'"
              >
                {{ i + 1 }}. {{ node.type === 'page' ? '页面' : '非页面' }}
              </option>
            </select>
          </label>
          <div id="logic-membership" tabindex="-1" class="logic-block form-stack">
            <h4>题目归属</h4>
            <label v-for="(q, i) in questions" :key="q.id" class="field">
              {{ i + 1 }}. {{ q.title || '未命名题目' }}
              <select
                :value="pageOf(q.id)"
                @change="assign(q.id, ($event.target as HTMLSelectElement).value)"
              >
                <option value="">未分配</option>
                <template v-for="(node, n) in logic.flow.nodes" :key="node.id">
                  <option v-if="node.type === 'page'" :value="node.id">{{ n + 1 }}. 页面</option>
                </template>
              </select>
            </label>
          </div>
          <section
            v-for="(node, index) in logic.flow.nodes"
            :id="`logic-node-${node.id}`"
            :key="node.id"
            tabindex="-1"
            class="logic-block form-stack"
          >
            <div class="actions between">
              <h4>
                {{ index + 1 }}.
                {{ node.type === 'page' ? '页面' : node.type === 'branch' ? '条件分支' : '结束' }}
              </h4>
              <div class="actions">
                <button
                  type="button"
                  class="text-button"
                  :disabled="index === 0"
                  @click="moveNode(index, -1)"
                >
                  上移
                </button>
                <button
                  type="button"
                  class="text-button"
                  :disabled="index === logic.flow.nodes.length - 1"
                  @click="moveNode(index, 1)"
                >
                  下移
                </button>
                <button
                  type="button"
                  class="text-button danger-text"
                  @click="logic.flow.nodes.splice(index, 1)"
                >
                  删除节点
                </button>
              </div>
            </div>
            <template v-if="node.type === 'page'">
              <p class="hint">包含 {{ node.question_ids.length }} 题</p>
              <button
                v-for="id in node.question_ids.filter((id) => !questions.some((q) => q.id === id))"
                :key="id"
                type="button"
                class="text-button danger-text"
                @click="node.question_ids = node.question_ids.filter((q) => q !== id)"
              >
                移除失效题目 {{ id }}
              </button>
              <label class="field">
                下一步
                <FlowTarget v-model="node.next" :nodes="logic.flow.nodes" :index="index" />
              </label>
            </template>
            <template v-else-if="node.type === 'branch'">
              <div v-for="(branch, b) in node.branches" :key="b" class="form-stack">
                <div class="actions between">
                  <strong>条件 {{ b + 1 }}</strong>
                  <div class="actions">
                    <button
                      type="button"
                      class="text-button"
                      :disabled="b === 0"
                      @click="node.branches.splice(b - 1, 0, node.branches.splice(b, 1)[0]!)"
                    >
                      提高优先级
                    </button>
                    <button type="button" class="text-button" @click="node.branches.splice(b, 1)">
                      删除条件
                    </button>
                  </div>
                </div>
                <ExpressionEditor
                  v-model="branch.when"
                  :questions="questions"
                  :variables="logic.variables"
                />
                <label class="field">
                  成立时前往
                  <FlowTarget v-model="branch.target" :nodes="logic.flow.nodes" :index="index" />
                </label>
              </div>
              <button
                type="button"
                class="button"
                :disabled="node.branches.length >= 16"
                @click="
                  node.branches.push({
                    when: { op: 'boolean', value: true },
                    target: node.otherwise,
                  })
                "
              >
                添加分支条件
              </button>
              <label class="field">
                所有条件均不成立时
                <FlowTarget v-model="node.otherwise" :nodes="logic.flow.nodes" :index="index" />
              </label>
            </template>
            <template v-else>
              <label class="field">
                结束方式
                <select v-model="node.outcome">
                  <option value="complete">完成填写，允许确认提交</option>
                  <option value="screen_out">筛选结束（仅可预览）</option>
                </select>
              </label>
              <p v-if="node.outcome === 'screen_out'" class="hint">
                包含筛选结束的问卷暂不可发布。
              </p>
            </template>
            <div class="actions">
              <span class="hint">在此节点后插入：</span>
              <button
                v-for="(label, type) in { page: '页面', branch: '分支', end: '结束' }"
                :key="type"
                type="button"
                class="button"
                :disabled="logic.flow.nodes.length >= 256"
                @click="addNode(index, type)"
              >
                {{ label }}
              </button>
            </div>
          </section>
          <button v-if="!logic.flow.nodes.length" type="button" class="button" @click="startFlow">
            重新建立流程
          </button>
        </template>
      </template>
    </template>
  </section>
</template>
