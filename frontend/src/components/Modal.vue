<!-- [入口在哪] 父视图触发模态框打开 -->
<!-- [用户在第 1 步后悔了怎么删/退] 顶栏右上关闭按钮、Esc 键、点击遮罩或底栏左侧取消按钮随时退出 -->
<!-- [主提交通道在哪] 模态框底部或表单末尾右侧的主操作按钮 -->
<!-- 原生对话框管理焦点，关闭时交还给触发控件。 -->
<script setup lang="ts">
import { onMounted, onBeforeUnmount, ref } from 'vue'
import Icon from './Icon.vue'
const props = withDefaults(
  defineProps<{ title: string; dismissible?: boolean; wide?: boolean }>(),
  { dismissible: true },
)
const emit = defineEmits<{ close: [] }>()
const dialog = ref<HTMLDialogElement>()
function close() {
  if (props.dismissible) emit('close')
}
function outside(event: MouseEvent) {
  const element = dialog.value
  if (!element || event.target !== element) return
  const rect = element.getBoundingClientRect()
  if (
    event.clientX < rect.left ||
    event.clientX > rect.right ||
    event.clientY < rect.top ||
    event.clientY > rect.bottom
  )
    close()
}
onMounted(() => dialog.value?.showModal())
onBeforeUnmount(() => dialog.value?.close())
</script>
<template>
  <dialog
    ref="dialog"
    class="design-dialog"
    :class="{ 'dialog-wide': wide }"
    :aria-label="title"
    @cancel.prevent="close"
    @click="outside"
  >
    <div class="dialog-header">
      <h2>{{ title }}</h2>
      <button
        v-if="dismissible"
        type="button"
        class="icon-button"
        aria-label="关闭对话框"
        @click="close"
      >
        <Icon name="close" />
      </button>
    </div>
    <div class="dialog-body"><slot /></div>
    <footer v-if="$slots.footer" class="dialog-footer"><slot name="footer" /></footer>
  </dialog>
</template>
