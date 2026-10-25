<script setup lang="ts">
import { computed } from "vue";
import { go, ui, type ViewName } from "../stores/ui";

type Tab = "online" | "library" | "mine";

/** 子页面归属到所属 Tab:详情页仍高亮资料库,设置/统计仍高亮我的 */
const TAB_OF_VIEW: Record<ViewName, Tab> = {
  online: "online",
  songs: "library",
  albums: "library",
  album: "library",
  artists: "library",
  playlists: "library",
  playlist: "library",
  search: "library",
  mine: "mine",
  settings: "mine",
  stats: "mine",
  trash: "mine",
};

const active = computed(() => TAB_OF_VIEW[ui.view]);

const tabs = [
  { id: "online", label: "在线", root: "online" },
  { id: "library", label: "资料库", root: "songs" },
  { id: "mine", label: "我的", root: "mine" },
] as const;

/** 点 Tab 始终回该 Tab 根页(再次点当前 Tab 也从详情退回根页) */
function switchTab(root: ViewName): void {
  go(root);
}
</script>

<template>
  <nav class="tabbar" aria-label="主导航">
    <button
      v-for="t in tabs"
      :key="t.id"
      class="tab"
      :class="{ on: active === t.id }"
      :aria-current="active === t.id ? 'page' : undefined"
      @click="switchTab(t.root)"
    >
      <svg v-if="t.id === 'online'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round">
        <circle cx="12" cy="12" r="8.3" />
        <path d="M3.7 12h16.6M12 3.7c2.5 2.3 3.8 5.1 3.8 8.3s-1.3 6-3.8 8.3c-2.5-2.3-3.8-5.1-3.8-8.3s1.3-6 3.8-8.3z" />
      </svg>
      <svg v-else-if="t.id === 'library'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <path d="M9.2 17.3V7l9-1.9v10.2" />
        <circle cx="6.9" cy="17.4" r="2.3" />
        <circle cx="15.9" cy="15.4" r="2.3" />
      </svg>
      <svg v-else viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round">
        <circle cx="12" cy="8.4" r="3.4" />
        <path d="M5.6 19c.8-3.3 3.3-4.9 6.4-4.9s5.6 1.6 6.4 4.9" />
      </svg>
      <span>{{ t.label }}</span>
    </button>
  </nav>
</template>

<style scoped>
.tabbar {
  flex: none;
  display: flex;
  align-items: stretch;
  padding-bottom: var(--tm-safe-bottom, 0px);
  background: var(--bar-bg);
  border-top: 1px solid var(--hairline);
  backdrop-filter: blur(24px) saturate(1.6);
  -webkit-backdrop-filter: blur(24px) saturate(1.6);
  z-index: 30;
}
.tab {
  flex: 1;
  min-height: 52px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 3px;
  color: var(--text-3);
  font-size: 10.5px;
  font-weight: 500;
  transition: color 0.18s ease, transform 0.2s var(--ease-spring);
}
.tab svg {
  width: 22px;
  height: 22px;
}
.tab.on {
  color: var(--accent);
  font-weight: 600;
}
.tab:active {
  transform: scale(0.94);
  transition-duration: 0.08s;
}
</style>
