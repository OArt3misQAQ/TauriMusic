<script setup lang="ts">
import { computed, onMounted, onUnmounted } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import Sidebar from "./components/Sidebar.vue";
import TopBar from "./components/TopBar.vue";
import NowPlaying from "./components/NowPlaying.vue";
import SettingsModal from "./components/SettingsModal.vue";
import AboutModal from "./components/AboutModal.vue";
import WelcomeModal from "./components/WelcomeModal.vue";
import PlaylistsView from "./views/PlaylistsView.vue";
import PlaylistDetailView from "./views/PlaylistDetailView.vue";
import NameModal from "./components/NameModal.vue";
import PlaylistPickerModal from "./components/PlaylistPickerModal.vue";
import ExportModal from "./components/ExportModal.vue";
import ContextMenu from "./components/ContextMenu.vue";
import HomeView from "./views/HomeView.vue";
import AlbumsView from "./views/AlbumsView.vue";
import AlbumDetailView from "./views/AlbumDetailView.vue";
import ArtistsView from "./views/ArtistsView.vue";
import SongsView from "./views/SongsView.vue";
import SearchView from "./views/SearchView.vue";
import StatsView from "./views/StatsView.vue";
import TrashView from "./views/TrashView.vue";
import { ui, initTheme, initSkin, initWelcome } from "./stores/ui";
import { exportState, initLibrary, initDragImport } from "./stores/library";
import { initShortcuts } from "./stores/shortcuts";
import { initStats } from "./stores/stats";
import { current } from "./stores/player";
import { nameDialog, picker } from "./stores/playlists";

const views = {
  home: HomeView,
  albums: AlbumsView,
  album: AlbumDetailView,
  artists: ArtistsView,
  songs: SongsView,
  search: SearchView,
  playlists: PlaylistsView,
  playlist: PlaylistDetailView,
  stats: StatsView,
  trash: TrashView,
} as const;

const view = computed(() => views[ui.view]);
/* 视图或其参数变化都重新触发页面过渡 */
const pageKey = computed(
  () => `${ui.view}|${ui.albumKey ?? ""}|${ui.artist ?? ""}|${ui.playlistId ?? ""}`,
);

const ambientImg = computed(() => {
  // 氛围背景全屏铺开,用大图档;模糊之下小图也够,但大图切歌时的淡入更细腻
  const src = current.value?.coverLarge ?? current.value?.cover;
  return src ? `url("${convertFileSrc(src)}")` : "";
});
const ambientKey = computed(() => current.value?.id ?? "none");

// 指针驱动的"液态光斑":玻璃面板的高光跟随光标流动(rAF 节流,仅玻璃模式更新)
let pointerRaf = 0;
function onPointerMove(e: PointerEvent): void {
  if (pointerRaf) return;
  pointerRaf = requestAnimationFrame(() => {
    pointerRaf = 0;
    if (ui.skin !== "glass") return;
    const root = document.documentElement;
    root.style.setProperty("--pointer-x", `${e.clientX}px`);
    root.style.setProperty("--pointer-y", `${e.clientY}px`);
  });
}

onMounted(() => {
  initTheme();
  initSkin();
  initWelcome();
  initStats();
  void initLibrary();
  void initDragImport();
  initShortcuts();
  window.addEventListener("pointermove", onPointerMove, { passive: true });
});
onUnmounted(() => {
  window.removeEventListener("pointermove", onPointerMove);
  cancelAnimationFrame(pointerRaf);
});
</script>

<template>
  <div class="app">
    <!-- SVG 折射滤镜:供玻璃卡片 backdrop-filter 引用(隐藏元素)。
         两次位移分别做横/纵"透镜"贴图:贴图 R/G 各只有一轴渐变(另一轴恒定 128 不位移),
         渐变集中在边缘 14%——中心清晰、边缘像真玻璃一样向外弯折。
         旧实现用 feTurbulence 噪声做位移,噪声颗粒感正是"塑料磨砂"的来源 -->
    <svg class="fx-defs" width="0" height="0" aria-hidden="true" focusable="false">
      <filter id="glass-refract" x="-20%" y="-20%" width="140%" height="140%" color-interpolation-filters="sRGB">
        <feImage
x="0" y="0" width="100%" height="100%" preserveAspectRatio="none" result="mapX"
          href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 100' preserveAspectRatio='none'%3E%3Cdefs%3E%3ClinearGradient id='g' x1='0' y1='0' x2='1' y2='0'%3E%3Cstop offset='0' stop-color='rgb(0,128,128)'/%3E%3Cstop offset='0.14' stop-color='rgb(128,128,128)'/%3E%3Cstop offset='0.86' stop-color='rgb(128,128,128)'/%3E%3Cstop offset='1' stop-color='rgb(255,128,128)'/%3E%3C/linearGradient%3E%3C/defs%3E%3Crect width='100' height='100' fill='url(%23g)'/%3E%3C/svg%3E" />
        <feDisplacementMap in="SourceGraphic" in2="mapX" scale="24" xChannelSelector="R" yChannelSelector="G" result="dx" />
        <feImage
x="0" y="0" width="100%" height="100%" preserveAspectRatio="none" result="mapY"
          href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 100' preserveAspectRatio='none'%3E%3Cdefs%3E%3ClinearGradient id='g' x1='0' y1='0' x2='0' y2='1'%3E%3Cstop offset='0' stop-color='rgb(128,0,128)'/%3E%3Cstop offset='0.14' stop-color='rgb(128,128,128)'/%3E%3Cstop offset='0.86' stop-color='rgb(128,128,128)'/%3E%3Cstop offset='1' stop-color='rgb(128,255,128)'/%3E%3C/linearGradient%3E%3C/defs%3E%3Crect width='100' height='100' fill='url(%23g)'/%3E%3C/svg%3E" />
        <feDisplacementMap in="dx" in2="mapY" scale="24" xChannelSelector="R" yChannelSelector="G" />
      </filter>
    </svg>

    <div v-if="ui.skin === 'glass'" class="ambient">
      <Transition name="amb">
        <div :key="ambientKey" class="ambient-img" :style="{ backgroundImage: ambientImg }"></div>
      </Transition>
      <div class="ambient-scrim"></div>
    </div>

    <div class="shell">
      <TopBar />
      <Sidebar />
      <main class="content">
        <Transition name="page" mode="out-in">
          <component :is="view" :key="pageKey" />
        </Transition>
      </main>
    </div>

    <Transition name="np">
      <NowPlaying v-if="ui.nowPlayingOpen && current" />
    </Transition>

    <Transition name="modal">
      <SettingsModal v-if="ui.settingsOpen" />
    </Transition>
    <Transition name="modal">
      <AboutModal v-if="ui.aboutOpen" />
    </Transition>
    <Transition name="modal">
      <WelcomeModal v-if="ui.welcomeOpen" />
    </Transition>

    <Transition name="modal">
      <NameModal v-if="nameDialog.open" />
    </Transition>
    <Transition name="modal">
      <PlaylistPickerModal v-if="picker.trackIds" />
    </Transition>
    <Transition name="modal">
      <ExportModal v-if="exportState.open" />
    </Transition>

    <ContextMenu />
  </div>
</template>

<style scoped>
.app {
  height: 100%;
  position: relative;
  overflow: hidden;
}
.shell {
  display: grid;
  grid-template-columns: 232px 1fr;
  grid-template-rows: 48px 1fr;
  height: 100%;
  /* 顶栏与侧栏共用的连续铬底,内容区以圆角纸面浮于其上(macOS 式) */
  background: var(--bg-2);
}
.content {
  position: relative;
  overflow: hidden;
  background: var(--bg);
  min-width: 0;
  border-top-left-radius: 14px;
  box-shadow:
    -1px -1px 0 var(--hairline),
    -8px -4px 20px rgba(0, 0, 0, 0.05);
}
.fx-defs {
  position: absolute;
  width: 0;
  height: 0;
}

/* 页面过渡:旧页快速虚化下沉,新页带模糊上浮落定 —— 玻璃的"流动感" */
.page-enter-active {
  transition:
    opacity 0.34s var(--ease-out-soft),
    transform 0.34s var(--ease-out-soft),
    filter 0.34s var(--ease-out-soft);
}
.page-leave-active {
  transition:
    opacity 0.16s ease,
    transform 0.16s ease,
    filter 0.16s ease;
}
.page-enter-from {
  opacity: 0;
  transform: translateY(14px) scale(0.992);
  filter: blur(5px);
}
.page-leave-to {
  opacity: 0;
  transform: scale(0.996);
  filter: blur(3px);
}

/* 全屏播放器:像一层玻璃从底部液化升起 */
.np-enter-active {
  transition:
    opacity 0.42s var(--ease-out-soft),
    transform 0.5s var(--ease-out-soft),
    filter 0.42s var(--ease-out-soft);
}
.np-leave-active {
  transition:
    opacity 0.26s ease,
    transform 0.3s ease,
    filter 0.26s ease;
}
.np-enter-from {
  opacity: 0;
  transform: translateY(48px) scale(1.015);
  filter: blur(8px);
}
.np-leave-to {
  opacity: 0;
  transform: translateY(30px) scale(0.99);
  filter: blur(5px);
}

/* 弹窗过渡:卡片弹簧上浮;遮罩用 background-color 淡入。
   遮罩不能动 opacity——overlay 挂着 backdrop-filter,opacity<1 会把元素隔离成
   backdrop root,模糊瞬间采不到背景,弹窗打开的第一帧窗口整个透底 */
.modal-enter-active {
  transition: background-color 0.28s var(--ease-out-soft);
}
.modal-leave-active {
  transition: background-color 0.2s ease;
}
.modal-leave-active :deep(.modal) {
  transition: opacity 0.2s ease;
}
.modal-leave-to :deep(.modal) {
  opacity: 0;
}
.modal-enter-active :deep(.modal) {
  transition: transform 0.38s var(--ease-spring);
}
.modal-enter-from {
  background-color: rgba(0, 0, 0, 0);
}
.modal-leave-to {
  background-color: rgba(0, 0, 0, 0);
}
.modal-enter-from :deep(.modal) {
  transform: translateY(16px) scale(0.965);
}
</style>
