<!-- [入口在哪] 任意未匹配路径；页面直接说明链接可能不完整，并给出两条出口。 -->
<!-- [用户在第 1 步后悔了怎么删/退] 左侧「返回上一页」就地回退，无需重新输入地址。 -->
<!-- [主提交通道在哪] 右侧唯一主按钮「返回工作台」回到应用主入口。 -->
<!-- 未匹配的路径展示明确出口。 -->
<script setup lang="ts">
import { RouterLink, useRouter } from 'vue-router'
const router = useRouter()
</script>
<template>
  <section class="public-container empty-state">
    <p class="eyebrow">404</p>
    <h1>这个页面不在这里</h1>
    <p>请确认链接是否完整，或返回工作台。</p>
    <div class="actions between">
      <button type="button" class="button" @click="router.back()">返回上一页</button>
      <RouterLink class="button button-primary" to="/">返回工作台</RouterLink>
    </div>
  </section>
</template>
