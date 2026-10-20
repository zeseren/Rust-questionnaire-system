<!-- [入口在哪] 应用外壳进入工作台/问卷/归档/设置；standalone 路由使用公共顶栏 -->
<!-- [用户在第 1 步后悔了怎么删/退] 公共顶栏常驻“返回工作台”，工作台顶栏与移动导航常驻退出登录；会话过期弹窗底栏左侧提供“退出并稍后处理” -->
<!-- [主提交通道在哪] 各视图页头右侧与固定底栏右侧的唯一主按钮；重新登录弹窗由表单主按钮提交 -->
<!-- 纸本与松烟墨布局共用浅深主题；会话过期时保留当前页面与未保存输入。 -->
<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { RouterLink, RouterView, useRoute, useRouter } from 'vue-router'
import { useAppearance, useAuth, useNotice, type Theme } from './stores'
import { message } from './api'
import Icons from './components/Icons.vue'
import Icon from './components/Icon.vue'
import Modal from './components/Modal.vue'
import AuthForm from './components/AuthForm.vue'
const route = useRoute()
const router = useRouter()
const appearance = useAppearance()
const auth = useAuth()
const notice = useNotice()
const mobileOpen = ref(false)
const mobileDialog = ref<HTMLDialogElement>()
const standalone = computed(() => Boolean(route.meta.standalone))
// 填写页与已登录用户都可在公共顶栏随时返回工作台，避免被困在独立流程中。
const showWorkspaceExit = computed(() => route.path.startsWith('/s/') || Boolean(auth.user))
const links = computed(() => [
  { to: '/', label: '工作台', icon: 'home' },
  { to: '/surveys', label: '我的问卷', icon: 'forms' },
  { to: '/archive', label: '已归档', icon: 'archive' },
  ...(auth.user?.role === 'admin' ? [{ to: '/admin', label: '平台管理', icon: 'people' }] : []),
  { to: '/settings', label: '账户设置', icon: 'person' },
])
const themes: { value: Theme; label: string; icon: string }[] = [
  { value: 'light', label: '浅色', icon: 'sun' },
  { value: 'dark', label: '深色', icon: 'moon' },
  { value: 'system', label: '系统', icon: 'desktop' },
]
function selected(path: string) {
  return path === '/' ? route.path === '/' : route.path.startsWith(path)
}
function expired() {
  if (route.meta.auth) auth.expired = true
}
window.addEventListener('session-expired', expired)
onBeforeUnmount(() => window.removeEventListener('session-expired', expired))
async function logout() {
  closeMobile()
  const failure = await router.push('/login')
  if (failure) return
  try {
    await auth.logout()
    notice.show('已退出登录。')
  } catch (error) {
    notice.show(message(error))
  }
}
// 会话过期弹窗不可用右上角关闭，必须保留“退出并稍后处理”逃生口。
async function dismissExpired() {
  await logout()
  auth.expired = false
}
function closeMobile() {
  mobileOpen.value = false
  mobileDialog.value?.close()
}
function openMobile() {
  mobileOpen.value = true
  mobileDialog.value?.showModal()
}
watch(() => route.fullPath, closeMobile)
</script>
<template>
  <Icons />
  <a class="skip-link" href="#main-content">跳至主要内容</a>
  <template v-if="!standalone">
    <aside class="sidebar desktop-sidebar" aria-label="工作区导航">
      <div class="brand-row">
        <RouterLink class="brand" to="/">
          <span class="brand-mark" aria-hidden="true">问</span>
          <span class="brand-title">
            <strong class="brand-name">问序工坊</strong>
            <span class="brand-caption">WENXU · STUDIO</span>
          </span>
        </RouterLink>
        <span class="edition-tag">DSL v1</span>
      </div>
      <div class="nav-caption">WORKSPACE · 工作空间</div>
      <nav class="nav-list">
        <RouterLink
          v-for="link in links"
          :key="link.to"
          class="nav-item"
          :class="{ 'is-active': selected(link.to) }"
          :to="link.to"
        >
          <Icon :name="link.icon" />
          {{ link.label }}
        </RouterLink>
      </nav>
      <div class="sidebar-card">
        <div class="sidebar-card-title">
          <span>规则引擎状态</span>
          <span class="dot-live">WASM 已就绪</span>
        </div>
        <p>前后端共享同一套 Rust 领域编译器，支持三值逻辑、精确大整数与确定性路径重建。</p>
      </div>
      <div class="sidebar-bottom">
        <div class="appearance-label">外观</div>
        <div class="theme-control" role="group" aria-label="选择外观主题">
          <button
            v-for="theme in themes"
            :key="theme.value"
            class="theme-option"
            :aria-pressed="appearance.preference === theme.value"
            @click="appearance.setTheme(theme.value)"
          >
            <Icon :name="theme.icon" />
            {{ theme.label }}
          </button>
        </div>
        <div class="profile">
          <span class="avatar">{{ auth.user?.username.slice(0, 1).toUpperCase() }}</span>
          <div class="grow truncate">
            <div class="profile-name truncate">{{ auth.user?.username }}</div>
            <div class="profile-description">
              {{ auth.user?.role === 'admin' ? '平台管理员' : '个人账号' }}
            </div>
          </div>
          <button class="icon-button" aria-label="退出登录" title="退出登录" @click="logout">
            <Icon name="logout" />
          </button>
        </div>
      </div>
    </aside>
    <dialog
      ref="mobileDialog"
      class="mobile-nav-dialog"
      aria-label="工作区导航"
      @close="mobileOpen = false"
      @cancel="mobileOpen = false"
    >
      <div class="dialog-header">
        <div class="brand">
          <span class="brand-mark" aria-hidden="true">问</span>
          <span class="brand-title">
            <strong class="brand-name">问序工坊</strong>
            <span class="brand-caption">WENXU · STUDIO</span>
          </span>
        </div>
        <button class="icon-button" aria-label="关闭导航" @click="closeMobile">
          <Icon name="close" />
        </button>
      </div>
      <nav class="nav-list">
        <RouterLink
          v-for="link in links"
          :key="link.to"
          class="nav-item"
          :class="{ 'is-active': selected(link.to) }"
          :to="link.to"
        >
          <Icon :name="link.icon" />
          {{ link.label }}
        </RouterLink>
        <button class="nav-item" @click="logout">
          <Icon name="logout" />
          退出登录
        </button>
      </nav>
    </dialog>
    <div class="app">
      <header class="topbar">
        <div class="breadcrumb">
          <button
            class="icon-button mobile-menu"
            aria-label="打开导航"
            :aria-expanded="mobileOpen"
            @click="openMobile"
          >
            <Icon name="menu" />
          </button>
          <Icon name="grid" />
          <span>个人工作区</span>
          <span class="breadcrumb-slash">/</span>
          <span class="breadcrumb-current">{{ route.meta.title }}</span>
        </div>
        <button
          class="icon-button"
          :aria-label="appearance.dark ? '切换浅色模式' : '切换深色模式'"
          @click="appearance.setTheme(appearance.dark ? 'light' : 'dark')"
        >
          <Icon :name="appearance.dark ? 'sun' : 'moon'" />
        </button>
      </header>
      <main id="main-content" class="main-content" tabindex="-1">
        <RouterView :key="String(route.name) + String(route.params.id || '')" />
      </main>
    </div>
  </template>
  <template v-else>
    <header class="public-header">
      <RouterLink class="brand" to="/">
        <span class="brand-mark" aria-hidden="true">问</span>
        <span class="brand-title">
          <strong class="brand-name">问序工坊</strong>
          <span class="brand-caption">WENXU · STUDIO</span>
        </span>
      </RouterLink>
      <div class="actions">
        <RouterLink v-if="showWorkspaceExit" class="text-button" to="/">
          <Icon name="home" />
          返回工作台
        </RouterLink>
        <button
          class="icon-button"
          :aria-label="appearance.dark ? '切换浅色模式' : '切换深色模式'"
          @click="appearance.setTheme(appearance.dark ? 'light' : 'dark')"
        >
          <Icon :name="appearance.dark ? 'sun' : 'moon'" />
        </button>
      </div>
    </header>
    <main id="main-content" tabindex="-1">
      <RouterView :key="String(route.name) + String(route.params.id || '')" />
    </main>
  </template>
  <div v-if="notice.text" class="toast" role="status">
    <Icon name="info" />
    <span>{{ notice.text }}</span>
    <button class="icon-button" aria-label="关闭提示" @click="notice.text = ''">
      <Icon name="close" />
    </button>
  </div>
  <Modal v-if="auth.expired && route.meta.auth" title="请重新登录" :dismissible="false">
    <p class="subtle mb">会话已过期，当前页面的输入仍然保留。登录后可继续操作。</p>
    <AuthForm :initial-username="auth.user?.username" />
    <template #footer>
      <button type="button" class="text-button danger-text" @click="dismissExpired">
        退出并稍后处理
      </button>
    </template>
  </Modal>
</template>
