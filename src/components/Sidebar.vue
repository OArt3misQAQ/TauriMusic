<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { ui, go, openPlaylist, type ViewName } from "../stores/ui";
import { lib } from "../stores/library";
import { statsEnabled } from "../stores/stats";
import { askCreatePlaylist, openPlaylistMenu } from "../stores/playlists";

const navItems = [
  { name: "albums", label: "专辑", icon: "disc" },
  { name: "artists", label: "艺人", icon: "artist" },
  { name: "songs", label: "歌曲", icon: "note" },
] as const;

function isActive(name: string): boolean {
  if (name === "albums") return ui.view === "albums" || ui.view === "album";
  if (name === "artists") return ui.view === "artists";
  return ui.view === name;
}

/** 搜索前的视图:清空搜索时回到原处而不是固定落回专辑页 */
let viewBeforeSearch: ViewName = "albums";
/** 输入框本地状态:180ms 防抖后再更新全局 ui.search,大曲库下避免每键击全库过滤 */
const searchInput = ref(ui.search);
let searchTimer = 0;

watch(searchInput, (v) => {
  window.clearTimeout(searchTimer);
  searchTimer = window.setTimeout(() => {
    if (v.trim()) {
      if (ui.view !== "search") viewBeforeSearch = ui.view;
      ui.view = "search";
    } else if (ui.view === "search") {
      ui.view = viewBeforeSearch;
    }
    ui.search = v;
  }, 180);
});

// 打开专辑/艺人等入口会清空 ui.search,同步回输入框
watch(
  () => ui.search,
  (v) => {
    if (v === "" && searchInput.value !== "") searchInput.value = "";
  },
);

const statusText = computed(() =>
  lib.scanning
    ? `正在扫描 ${lib.scanTotal > 0 ? `${lib.scanCurrent}/${lib.scanTotal}` : "…"}`
    : lib.importStatus,
);
</script>

<template>
  <aside class="sidebar">
    <div class="search">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round">
        <circle cx="11" cy="11" r="6.5" />
        <path d="m16.2 16.2 4.3 4.3" />
      </svg>
      <input v-model="searchInput" type="text" placeholder="搜索" spellcheck="false" />
    </div>

    <div class="section-label">资料库</div>
    <nav>
      <button
        v-for="item in navItems"
        :key="item.name"
        class="nav-item"
        :class="{ active: isActive(item.name) }"
        @click="go(item.name)"
      >
        <svg v-if="item.icon === 'disc'" class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
          <circle cx="12" cy="12" r="8.2" />
          <circle cx="12" cy="12" r="2.1" fill="currentColor" stroke="none" />
        </svg>
        <svg v-else-if="item.icon === 'artist'" class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round">
          <circle cx="12" cy="8.4" r="3.4" />
          <path d="M5.6 19c.8-3.3 3.3-4.9 6.4-4.9s5.6 1.6 6.4 4.9" />
        </svg>
        <svg v-else class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
          <path d="M9.2 17.3V7l9-1.9v10.2" />
          <circle cx="6.9" cy="17.4" r="2.3" />
          <circle cx="15.9" cy="15.4" r="2.3" />
        </svg>
        <span>{{ item.label }}</span>
      </button>
      <button
        class="nav-item pl-item pl-all"
        :class="{ active: ui.view === 'playlists' }"
        @click="go('playlists')"
      >
        <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round">
          <path d="M4 6.5h11M4 12h11M4 17.5h7" />
          <circle cx="18.5" cy="15.5" r="2.2" />
          <path d="M20.7 15.5V9l-3.4.9" />
        </svg>
        <span class="pl-text">全部播放列表</span>
      </button>
      <!-- 听歌统计:总开关关闭时入口一并隐藏(设置 → 统计与隐私) -->
      <button v-if="statsEnabled" class="nav-item" :class="{ active: ui.view === 'stats' }" @click="go('stats')">
        <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round">
          <path d="M5 20v-8M12 20V5M19 20v-5" />
        </svg>
        <span>听歌统计</span>
      </button>
    </nav>

    <div class="section-label pl-head">
      播放列表
      <button class="pl-add" title="新建播放列表" @click="askCreatePlaylist">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
          <path d="M12 5.5v13M5.5 12h13" />
        </svg>
      </button>
    </div>
    <div class="pl-list">
      <button
        v-for="p in lib.playlists"
        :key="p.id"
        class="nav-item pl-item"
        :class="{ active: ui.view === 'playlist' && ui.playlistId === p.id }"
        :title="p.name"
        @click="openPlaylist(p.id)"
        @contextmenu="openPlaylistMenu($event, p)"
      >
        <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round">
          <path d="M4 6.5h11M4 12h11M4 17.5h7" />
          <circle cx="18.5" cy="15.5" r="2.2" />
          <path d="M20.7 15.5V9l-3.4.9" />
        </svg>
        <span class="pl-text">{{ p.name }}</span>
      </button>
    </div>

    <div class="spacer" />

    <div v-if="statusText" class="status">{{ statusText }}</div>

    <div class="actions">
      <button
        class="action"
        :class="{ active: ui.view === 'trash' }"
        title="回收站:删除的音乐在这里保留 30 天,可还原"
        @click="go('trash')"
      >
        <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
          <path d="M4.5 7h15M9.5 7V5.2c0-.66.54-1.2 1.2-1.2h2.6c.66 0 1.2.54 1.2 1.2V7M6.5 7l.8 11.3c.06.77.7 1.2 1.4 1.2h6.6c.7 0 1.34-.43 1.4-1.2L17.5 7" />
          <path d="M10 11v5M14 11v5" />
        </svg>
        <span>回收站</span>
        <span v-if="lib.trash.length" class="badge">{{ lib.trash.length }}</span>
      </button>
      <button class="action" title="设置" @click="ui.settingsOpen = true">
        <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round">
          <path d="M5 7.5h7M16.5 7.5H19M5 12h3M11.5 12H19M5 16.5h7M16.5 16.5H19" />
          <circle cx="14" cy="7.5" r="2" />
          <circle cx="9.5" cy="12" r="2" />
          <circle cx="14" cy="16.5" r="2" />
        </svg>
        <span>设置</span>
      </button>
      <button class="action" title="关于 TauriMusic" @click="ui.aboutOpen = true">
        <svg class="ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round">
          <circle cx="12" cy="12" r="8.2" />
          <path d="M12 11.2v4.6" />
          <circle cx="12" cy="8" r="1" fill="currentColor" stroke="none" />
        </svg>
        <span>关于</span>
      </button>
    </div>
  </aside>
</template>

<style scoped>
.sidebar {
  grid-row: 2;
  grid-column: 1;
  display: flex;
  flex-direction: column;
  background: transparent; /* 与顶栏共用 shell 的连续铬底 */
  padding: 14px 12px 14px;
  gap: 4px;
  min-height: 0;
}

.search {
  display: flex;
  align-items: center;
  gap: 7px;
  background: var(--pill-bg);
  border-radius: 8px;
  padding: 6px 10px;
  color: var(--text-2);
  margin-bottom: 14px;
  flex: none;
}
.search svg {
  width: 15px;
  height: 15px;
  flex: none;
}
.search input {
  flex: 1;
  min-width: 0;
  border: none;
  outline: none;
  background: transparent;
  color: var(--text);
  font-size: 13px;
}
.search input::placeholder {
  color: var(--text-3);
}

.section-label {
  font-size: 11px;
  font-weight: 700;
  color: var(--text-3);
  padding: 0 10px 6px;
  letter-spacing: 0.8px;
  flex: none;
}

nav {
  display: flex;
  flex-direction: column;
  gap: 2px;
  flex: none;
}

.nav-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 6px 10px;
  border-radius: 7px;
  font-size: 13.5px;
  font-weight: 500;
  color: var(--text);
  text-align: left;
  transition: background 0.25s var(--ease-out-soft), transform 0.35s var(--ease-spring);
}
.nav-item:hover {
  background: var(--hover);
}
.nav-item:active {
  transform: scale(0.98);
  transition-duration: 0.1s;
}
.nav-item.active {
  background: var(--active);
  color: var(--text);
}
.nav-item .ico {
  width: 18px;
  height: 18px;
  flex: none;
  color: var(--text-2);
}
.nav-item.active .ico {
  color: var(--text);
}
/* 玻璃皮肤下激活项红字红图标(Apple Music 式),非激活保持灰 */
html.glass .nav-item.active .ico {
  color: var(--accent);
}

.pl-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding-right: 6px;
  margin-top: 12px;
}
.pl-add {
  width: 20px;
  height: 20px;
  border-radius: 6px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-3);
  transition: background 0.2s ease, color 0.2s ease;
}
.pl-add:hover {
  background: var(--hover);
  color: var(--text);
}
.pl-add svg {
  width: 13px;
  height: 13px;
}
.pl-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
  flex: none;
  max-height: 30vh;
  overflow-y: auto;
  /* macOS 叠加式滚动条:平时隐藏 */
}
.pl-list::-webkit-scrollbar {
  width: 8px;
}
.pl-list::-webkit-scrollbar-thumb {
  background: transparent;
  border-radius: 4px;
  border: 2px solid transparent;
  background-clip: content-box;
}
.pl-list:hover::-webkit-scrollbar-thumb {
  background: var(--scrollbar);
  background-clip: content-box;
}
.pl-item .ico {
  color: var(--text-3);
}
.pl-item.active .ico {
  color: var(--text);
}
.pl-text {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.pl-all {
  margin-top: 2px;
}

.spacer {
  flex: 1;
}

.status {
  font-size: 12px;
  color: var(--text-2);
  padding: 0 10px 8px;
  flex: none;
}

.actions {
  display: flex;
  flex-direction: column;
  gap: 2px;
  flex: none;
  padding-top: 8px;
  border-top: 1px solid var(--hairline);
}

.action {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 6px 10px;
  border-radius: 7px;
  font-size: 13px;
  color: var(--text-2);
  text-align: left;
  transition: background 0.25s var(--ease-out-soft), color 0.2s ease, transform 0.35s var(--ease-spring);
}
.action:hover {
  background: var(--hover);
  color: var(--text);
}
.action.active {
  background: var(--active);
  color: var(--text);
}
.action.active .ico {
  color: var(--text);
}
html.glass .action.active .ico {
  color: var(--accent);
}
.action .badge {
  margin-left: auto;
  font-size: 10.5px;
  font-weight: 700;
  color: var(--text-3);
  background: var(--pill-bg);
  border-radius: 999px;
  padding: 1px 7px;
}
.action:active {
  transform: scale(0.97);
  transition-duration: 0.1s;
}
.action .ico {
  width: 17px;
  height: 17px;
  flex: none;
}
</style>
