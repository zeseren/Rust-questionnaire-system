<!-- 图标共享本地符号，不依赖外部图片请求。 -->
<script setup lang="ts">
defineProps<{ name: string }>()
</script>
<template>
  <svg class="icon" aria-hidden="true"><use :href="`#i-${name}`" /></svg>
</template>
