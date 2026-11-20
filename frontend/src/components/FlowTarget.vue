<!-- 跳转只选择后续节点；失效引用保持可见，等待作者明确修正。 -->
<script setup lang="ts">
import type { FlowNode } from '../types'
import { nodeTypeNames } from '../flow-tools'
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
      {{ nodeTypeNames[node.type] }}
    </option>
  </select>
</template>
