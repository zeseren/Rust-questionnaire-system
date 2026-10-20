<!-- [入口在哪] /login 与 /register 共用此页；卡片眉题行右侧即可在登录/注册间切换。 -->
<!-- [用户在第 1 步后悔了怎么删/退] 卡片底部常驻「← 返回首页」，不依赖登录态，随时可退出认证流程。 -->
<!-- [主提交通道在哪] 表单内唯一主按钮「登录/创建账号」；切换入口与返回首页均为低权重文本链接。 -->
<!-- 注册后使用同一登录流程；返回地址仅允许应用内部路径。 -->
<script setup lang="ts">
import { computed } from 'vue'
import { RouterLink, useRoute, useRouter } from 'vue-router'
import AuthForm from '../components/AuthForm.vue'
import ErrorNotice from '../components/ErrorNotice.vue'
import Icon from '../components/Icon.vue'
import { safeReturn } from '../router'
import { useAuth } from '../stores'
const route = useRoute()
const router = useRouter()
const auth = useAuth()
const register = computed(() => route.name === 'register')
function success() {
  router.replace(safeReturn(route.query.redirect))
}
</script>
<template>
  <div class="auth-layout">
    <section class="auth-intro">
      <p class="eyebrow">提问 · 倾听 · 发现</p>
      <h1>
        让每一个想法，
        <br />
        都被听见。
      </h1>
      <p class="subtle">
        从一个好问题开始，把真实的声音，
        <br />
        变成下一步的方向。
      </p>
      <div class="hero-art" aria-hidden="true">
        <div class="metro-tile metro-tile-main">
          <Icon name="forms" />
          <span class="metro-tile-label">提问</span>
        </div>
        <div class="metro-tile metro-tile-listen">
          <Icon name="people" />
          <span class="metro-tile-label">倾听</span>
        </div>
        <div class="metro-tile metro-tile-insight">
          <Icon name="chart" />
          <span class="metro-tile-label">发现</span>
        </div>
      </div>
    </section>
    <section class="auth-card">
      <div class="actions between">
        <p class="eyebrow">欢迎来到问序</p>
        <RouterLink
          class="text-button"
          :to="{ name: register ? 'login' : 'register', query: { redirect: route.query.redirect } }"
        >
          {{ register ? '去登录' : '创建账号' }}
        </RouterLink>
      </div>
      <h2>{{ register ? '创建你的账号' : '欢迎回来' }}</h2>
      <p class="subtle mb">
        {{ register ? '让你的第一个好问题，从这里开始。' : '登录后，继续你未完成的发现。' }}
      </p>
      <ErrorNotice :message="auth.restoreError" />
      <AuthForm :mode="register ? 'register' : 'login'" @success="success" />
      <div class="actions mt">
        <RouterLink class="text-button" to="/">← 返回首页</RouterLink>
      </div>
    </section>
  </div>
</template>
