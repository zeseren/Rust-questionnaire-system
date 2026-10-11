<!-- 跳转只选择后续节点；失效引用保持可见，等待作者明确修正。 -->
<script setup lang="ts">
import type { FlowNode } from '../types'
defineProps<{ nodes: FlowNode[]; index: number }>()
const target = defineModel<string>({ required: true })
</script>
<template>
  <select v-model="target">
    <option v-if="!nodes.slice(index + 1).some((n) => n.id === target)" :value="target">
      请选择后续节点（当前出口无效）
    </option>
    <option v-for="(node, offset) in nodes.slice(index + 1)" :key="node.id" :value="node.id">
      {{ index + offset + 2 }}.
      {{ node.type === 'page' ? '页面' : node.type === 'branch' ? '条件分支' : '结束' }}
    </option>
  </select>
</template>
