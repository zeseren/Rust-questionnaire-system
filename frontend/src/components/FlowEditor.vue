<!-- [入口在哪] 规则与跳题工作区的「页面流程」分类内，从概览或节点卡片直接进入编辑。 -->
<!-- [用户在第 1 步后悔了怎么删/退] 顶部右侧常驻低饱和「移除页面流程」幽灵按钮可整体退回单页；每个节点右侧的「删除节点」悬停变红并二次确认，随时可撤销单点改动。 -->
<!-- [主提交通道在哪] 流程编辑不设主按钮，修改即时写回；顶部唯一主按钮仍是编辑器固定栏的「发布问卷」。 -->
<!-- 页面顺序和出口由作者显式配置，结构变化后保留失效引用供定位修复。 -->
<script lang="ts">
// 分支对象会因提高优先级而换位，用对象身份分配稳定 key，避免 Vue 复用错误的 DOM。
const branchKeys = new WeakMap<object, number>()
let branchKeySeq = 0
</script>
<script setup lang="ts">
import { computed, nextTick } from 'vue'
import { insertFlowNode } from '../flow-tools'
import { newId } from '../uuid'
import type { ComputedVariable, FlowBranch, FlowDefinition, FlowNode, Question } from '../types'
import ConditionBuilder from './ConditionBuilder.vue'
import FlowTarget from './FlowTarget.vue'
import FlowOverview from './FlowOverview.vue'
import SearchSelect from './SearchSelect.vue'
import Icon from './Icon.vue'
const flow = defineModel<FlowDefinition | null | undefined>({ required: true })
const props = defineProps<{ questions: Question[]; variables: ComputedVariable[] }>()
function start(perQuestion = false) {
  if (!props.questions.length) return
  const end = newId()
  const groups = perQuestion
    ? props.questions.map((question) => [question.id])
    : [props.questions.map((question) => question.id)]
  const pages = groups.map((question_ids) => ({
    type: 'page' as const,
    id: newId(),
    question_ids,
    next: end,
  }))
  pages.forEach((page, index) => {
    page.next = pages[index + 1]?.id ?? end
  })
  const firstPage = pages[0]
  if (!firstPage) return
  flow.value = {
    entry: firstPage.id,
    nodes: [...pages, { type: 'end', id: end, outcome: 'complete' }],
  }
}
async function locate(id: string) {
  await nextTick()
  document.getElementById(`logic-node-${id}`)?.focus()
}
function addNode(index: number, type: FlowNode['type']) {
  if (!flow.value) return
  // 结束节点之后没有可接管的出口，插入会被拒绝。
  const id = insertFlowNode(flow.value, index, type)
  if (!id) return
  void locate(id)
}
function move(index: number, direction: number) {
  if (!flow.value) return
  const to = index + direction
  if (to < 0 || to >= flow.value.nodes.length) return
  const [moved] = flow.value.nodes.splice(index, 1)
  if (!moved) return
  flow.value.nodes.splice(to, 0, moved)
}
function raiseBranch(branches: FlowBranch[], index: number) {
  const [moved] = branches.splice(index, 1)
  if (!moved) return
  branches.splice(index - 1, 0, moved)
}
function pageNodes() {
  return (flow.value?.nodes ?? []).filter(
    (node): node is Extract<FlowNode, { type: 'page' }> => node.type === 'page',
  )
}
/** 页面拼接后的题目顺序必须等于全局题目顺序：只有移到相邻页面的接缝处才合法。 */
function assignablePages(questionId: string) {
  const pages = pageNodes()
  const current = pages.find((node) => node.question_ids.includes(questionId))
  if (!current) return new Set<string>()
  const legal = new Set([current.id])
  // 移走页面里唯一的题目会留下空页面，Rust 流程校验会拒绝。
  if (current.question_ids.length > 1) {
    const index = pages.indexOf(current)
    const position = current.question_ids.indexOf(questionId)
    const previous = pages[index - 1]
    const next = pages[index + 1]
    if (position === 0 && previous) legal.add(previous.id)
    if (position === current.question_ids.length - 1 && next) legal.add(next.id)
  }
  return legal
}
function assignOptions(questionId: string) {
  const legal = assignablePages(questionId)
  return [
    { value: '', label: '未分配', disabled: true },
    ...(flow.value?.nodes ?? []).flatMap((node, index) =>
      node.type === 'page'
        ? [{ value: node.id, label: `${index + 1}. 页面`, disabled: !legal.has(node.id) }]
        : [],
    ),
  ]
}
function assign(questionId: string, pageId: string) {
  if (!assignablePages(questionId).has(pageId)) return
  const pages = pageNodes()
  const current = pages.find((node) => node.question_ids.includes(questionId))
  const target = pages.find((node) => node.id === pageId)
  if (!current || !target || current === target) return
  const order = new Map(props.questions.map((question, position) => [question.id, position]))
  current.question_ids = current.question_ids.filter((id) => id !== questionId)
  target.question_ids.push(questionId)
  target.question_ids.sort((a, b) => (order.get(a) ?? -1) - (order.get(b) ?? -1))
}
function questionTitle(id: string) {
  const question = props.questions.find((item) => item.id === id)
  if (!question) return '题目已删除'
  return question.title || '未命名题目'
}
function branchKey(branch: FlowBranch) {
  let key = branchKeys.get(branch)
  if (key === undefined) {
    key = ++branchKeySeq
    branchKeys.set(branch, key)
  }
  return key
}
function pageOf(id: string) {
  return (
    flow.value?.nodes.find((node) => node.type === 'page' && node.question_ids.includes(id))?.id ??
    ''
  )
}
const preceding = computed(() => {
  const seen = new Set<string>()
  return (flow.value?.nodes ?? []).map((node) => {
    const available = props.questions.filter((question) => seen.has(question.id))
    if (node.type === 'page') for (const id of node.question_ids) seen.add(id)
    return available
  })
})
function precedingQuestions(index: number) {
  return preceding.value[index] ?? []
}
function removeNode(index: number) {
  if (!flow.value) return
  if (!window.confirm('确定删除该流程节点吗？')) return
  flow.value.nodes.splice(index, 1)
}
function removeFlow() {
  if (
    window.confirm('移除分页与分支后，所有可见题目将在同一页填写。题目规则和变量会保留，是否继续？')
  )
    flow.value = null
}
</script>
<template>
  <div class="form-stack">
    <div class="actions between">
      <div>
        <h3>页面流程</h3>
        <p class="hint">组织填写步骤，让不同的回答走向合适的问题。</p>
      </div>
      <button v-if="flow" type="button" class="text-button danger-text" @click="removeFlow">
        移除页面流程
      </button>
    </div>
    <div v-if="!flow" class="logic-empty">
      <span class="template-icon teal"><Icon name="grid" /></span>
      <h3>把问卷分成轻松的几个步骤</h3>
      <p class="hint">目前全部可见题目在同一页。可以先分页，再添加条件分支。</p>
      <div class="actions">
        <button type="button" class="button" :disabled="!questions.length" @click="start(true)">
          每题一页
        </button>
        <button type="button" class="button" :disabled="!questions.length" @click="start()">
          从一个页面开始
        </button>
      </div>
    </div>
    <template v-else>
      <FlowOverview :flow="flow" :questions="questions" @select="locate" />
      <p class="hint">点击概览定位节点。出口只能前往后续节点；多个出口可指向同一页面实现汇合。</p>
      <label class="field">
        入口页面
        <SearchSelect
          v-model="flow.entry"
          label="入口页面"
          :options="
            flow.nodes.flatMap((node, index) =>
              node.type === 'page' ? [{ value: node.id, label: `${index + 1}. 页面` }] : [],
            )
          "
        />
      </label>
      <section id="logic-membership" tabindex="-1" class="logic-block form-stack">
        <div>
          <h3>题目归属</h3>
          <p class="hint">每道题属于一个页面，页面中的题目次序与题目设计保持一致。</p>
        </div>
        <label v-for="(question, index) in questions" :key="question.id" class="membership-row">
          <span>
            <span class="accent">{{ String(index + 1).padStart(2, '0') }}</span>
            {{ question.title || '未命名题目' }}
          </span>
          <SearchSelect
            :model-value="pageOf(question.id)"
            :label="`第 ${index + 1} 题归属页面`"
            :options="assignOptions(question.id)"
            @update:model-value="assign(question.id, $event)"
          />
        </label>
      </section>
      <section
        v-for="(node, index) in flow.nodes"
        :id="`logic-node-${node.id}`"
        :key="node.id"
        tabindex="-1"
        class="logic-block form-stack flow-node-detail"
        :class="`node-${node.type}`"
      >
        <div class="actions between">
          <h3>
            <span class="node-index">{{ index + 1 }}</span>
            {{ node.type === 'page' ? '页面' : node.type === 'branch' ? '条件分支' : '结束' }}
          </h3>
          <div class="actions">
            <button
              type="button"
              class="icon-button"
              :disabled="index === 0"
              :aria-label="`上移节点 ${index + 1}`"
              @click="move(index, -1)"
            >
              <Icon name="up" />
            </button>
            <button
              type="button"
              class="icon-button"
              :disabled="index === flow.nodes.length - 1"
              :aria-label="`下移节点 ${index + 1}`"
              @click="move(index, 1)"
            >
              <Icon name="down" />
            </button>
            <button
              type="button"
              class="icon-button danger-text"
              style="margin-left: 8px"
              :aria-label="`删除节点 ${index + 1}`"
              @click="removeNode(index)"
            >
              <Icon name="trash" />
            </button>
          </div>
        </div>
        <template v-if="node.type === 'page'">
          <div class="page-question-chips">
            <span
              v-for="questionId in node.question_ids"
              :key="questionId"
              class="page-question-chip"
            >
              {{ questionTitle(questionId) }}
            </span>
            <span v-if="!node.question_ids.length" class="hint">
              空页面，请在题目归属中分配题目。
            </span>
          </div>
          <button
            v-for="id in node.question_ids.filter(
              (id) => !questions.some((question) => question.id === id),
            )"
            :key="id"
            type="button"
            class="text-button danger-text"
            @click="node.question_ids = node.question_ids.filter((questionId) => questionId !== id)"
          >
            移除失效题目引用
          </button>
          <label class="field">
            填写后前往
            <FlowTarget v-model="node.next" :nodes="flow.nodes" :index="index" />
          </label>
        </template>
        <template v-else-if="node.type === 'branch'">
          <p class="hint">从上到下检查条件，第一个成立的出口生效。</p>
          <section
            v-for="(branch, branchIndex) in node.branches"
            :key="branchKey(branch)"
            class="branch-condition form-stack"
          >
            <div class="actions between">
              <strong>条件 {{ branchIndex + 1 }}</strong>
              <div class="actions">
                <button
                  type="button"
                  class="text-button"
                  :disabled="branchIndex === 0"
                  @click="raiseBranch(node.branches, branchIndex)"
                >
                  提高优先级
                </button>
                <button
                  type="button"
                  class="text-button danger-text"
                  :disabled="node.branches.length <= 1"
                  @click="node.branches.splice(branchIndex, 1)"
                >
                  删除条件
                </button>
              </div>
            </div>
            <ConditionBuilder
              v-model="branch.when"
              :questions="precedingQuestions(index)"
              :variables="variables"
            />
            <label class="field">
              成立时前往
              <FlowTarget v-model="branch.target" :nodes="flow.nodes" :index="index" />
            </label>
          </section>
          <button
            type="button"
            class="text-button accent"
            :disabled="node.branches.length >= 16"
            @click="
              node.branches.push({ when: { op: 'boolean', value: true }, target: node.otherwise })
            "
          >
            <Icon name="plus" />
            添加分支条件
          </button>
          <label class="field">
            以上条件均不成立时
            <FlowTarget v-model="node.otherwise" :nodes="flow.nodes" :index="index" />
          </label>
        </template>
        <template v-else>
          <label class="field">
            结束方式
            <select v-model="node.outcome">
              <option value="complete">完成填写，允许确认提交</option>
              <option value="screen_out">筛选结束（落库为甄别记录，计入总收卷）</option>
            </select>
          </label>
          <p v-if="node.outcome === 'screen_out'" class="hint">
            筛选结束同样返回回执并计入总收卷，但不计入各题统计与分母。
          </p>
        </template>
        <div v-if="node.type !== 'end'" class="node-insert actions">
          <span class="hint">在后面插入</span>
          <button
            v-for="(label, type) in { page: '页面', branch: '分支', end: '结束' }"
            :key="type"
            type="button"
            class="text-button"
            :disabled="flow.nodes.length >= 256"
            @click="addNode(index, type)"
          >
            <Icon name="plus" />
            {{ label }}
          </button>
        </div>
      </section>
      <button v-if="!flow.nodes.length" type="button" class="button" @click="start()">
        重新建立流程
      </button>
    </template>
  </div>
</template>
