<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { player, current, toggle, next, prev, setVolume } from "../stores/player";
import ModeIcon from "./ModeIcon.vue";
import { ui } from "../stores/ui";

const appWin = getCurrentWindow();
const isMax = ref(false);
let unlisten: (() => void) | null = null;

onMounted(async () => {
  isMax.value = await appWin.isMaximized().catch(() => false);
  unlisten = await appWin.onResized(async () => {
    isMax.value = await appWin.isMaximized().catch(() => isMax.value);
  });
});
onUnmounted(() => unlisten?.());

const coverUrl = computed(() => (current.value?.cover ? convertFileSrc(current.value.cover) : null));
const progress = computed(() => (player.duration > 0 ? (player.position / player.duration) * 100 : 0));
const volFill = computed(() => `${player.volume * 100}%`);
</script>

<template>
  <!-- deep: 整个顶栏都是拖拽区,按钮/滑杆等可点击控件自动豁免(Tauri 2.12 drag.js 规则) -->
  <header class="topbar" data-tauri-drag-region="deep">
    <div class="brand">
      <span class="logo">
        <svg viewBox="0 0 24 24">
          <!-- 迷你黑胶:与应用图标「墨夜胶片」同一几何语言,中孔透出陶土底色 -->
          <circle cx="12" cy="12" r="8.6" fill="none" stroke="#fff" stroke-width="2" />
          <circle cx="12" cy="12" r="3.4" fill="#fff" />
          <circle cx="12" cy="12" r="1.2" style="fill: var(--accent)" />
        </svg>
      </span>
      <span class="name">TauriMusic</span>
    </div>

    <div class="center">
      <div class="controls">
        <ModeIcon class="t-btn" hint-dir="below" />
        <button class="t-btn big" title="上一曲" :disabled="!current" @click="prev()">
          <svg viewBox="0 0 24 24">
            <path d="M7 5.8v12.4" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" />
            <path d="M18.5 6.3v11.4L9.4 12z" fill="currentColor" />
          </svg>
        </button>
        <button class="t-btn play" :title="player.playing ? '暂停' : '播放'" @click="toggle()">
          <svg v-if="!player.playing" viewBox="0 0 24 24">
            <path d="M8.2 5.5v13L19 12z" fill="currentColor" />
          </svg>
          <svg v-else viewBox="0 0 24 24">
            <path d="M7.6 5.5h3.3v13H7.6zM13.2 5.5h3.3v13h-3.3z" fill="currentColor" />
          </svg>
        </button>
        <button class="t-btn big" title="下一曲" :disabled="!current" @click="next()">
          <svg viewBox="0 0 24 24">
            <path d="M17 5.8v12.4" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" />
            <path d="M5.5 6.3v11.4L14.6 12z" fill="currentColor" />
          </svg>
        </button>
      </div>

      <button class="np-pill" :disabled="!current" :title="current ? '展开播放页' : '未在播放'" @click="current && (ui.nowPlayingOpen = true)">
        <span class="p-cover">
          <img v-if="coverUrl" :src="coverUrl" alt="" />
          <span v-else class="ph">♪</span>
        </span>
        <span v-if="current" class="p-meta">
          <span class="p-title">{{ current.title }}</span>
          <span class="p-artist">{{ current.artist }}</span>
        </span>
        <span v-else class="p-meta"><span class="p-title muted">未在播放</span></span>
        <span class="p-line" :style="{ width: progress + '%' }"></span>
      </button>
    </div>

    <div class="right">
      <svg class="vol-ico" viewBox="0 0 24 24">
        <path d="M11.2 4.8 6.8 8.3H4v7.4h2.8l4.4 3.5z" fill="currentColor" />
        <path d="M14.5 9.3a3.9 3.9 0 0 1 0 5.4M17 7a7.2 7.2 0 0 1 0 10" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" />
      </svg>
      <input
        class="range vol"
        type="range"
        min="0"
        max="1"
        step="0.01"
        :value="player.volume"
        :style="{ '--fill': volFill }"
        @input="setVolume(Number(($event.target as HTMLInputElement).value))"
      />
    </div>

    <div class="caps">
      <button class="cap" title="最小化" @click="appWin.minimize()">
        <svg viewBox="0 0 24 24"><path d="M5 12h14" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" /></svg>
      </button>
      <button class="cap" :title="isMax ? '还原' : '最大化'" @click="appWin.toggleMaximize()">
        <svg v-if="!isMax" viewBox="0 0 24 24">
          <rect x="6.5" y="6.5" width="11" height="11" rx="1.5" fill="none" stroke="currentColor" stroke-width="1.4" />
        </svg>
        <svg v-else viewBox="0 0 24 24">
          <rect x="5.5" y="8.5" width="10" height="10" rx="1.5" fill="none" stroke="currentColor" stroke-width="1.4" />
          <path d="M8.5 5.5h8a2 2 0 0 1 2 2v8" fill="none" stroke="currentColor" stroke-width="1.4" />
        </svg>
      </button>
      <button class="cap close" title="关闭" @click="appWin.close()">
        <svg viewBox="0 0 24 24"><path d="m6 6 12 12M18 6 6 18" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" /></svg>
      </button>
    </div>
  </header>
</template>

<style scoped>
.topbar {
  grid-column: 1 / 3;
  position: relative;
  z-index: 100;
  display: flex;
  align-items: center;
  gap: 16px;
  height: 48px;
  padding-left: 16px;
  background: transparent; /* 与侧栏共用 shell 的连续铬底 */
  border-bottom: 1px solid var(--hairline);
  user-select: none;
}

.brand {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 200px;
  flex: none;
}
.logo {
  width: 22px;
  height: 22px;
  border-radius: 6px;
  background: var(--accent);
  display: flex;
  align-items: center;
  justify-content: center;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.25);
}
.logo svg {
  width: 15px;
  height: 15px;
}
.name {
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
}

.center {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 18px;
}
.controls {
  display: flex;
  align-items: center;
  gap: 14px;
}
.t-btn {
  position: relative;
  width: 26px;
  height: 26px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-2);
  transition: color 0.15s ease, transform 0.3s var(--ease-spring);
}
.t-btn:hover:not(:disabled) {
  color: var(--text);
}
.t-btn:active:not(:disabled) {
  transform: scale(0.86);
  transition-duration: 0.08s;
}
.t-btn:disabled {
  opacity: 0.35;
  cursor: default;
}
.t-btn.on {
  color: var(--accent);
}
.t-btn svg {
  width: 19px;
  height: 19px;
}
.t-btn.play {
  width: 30px;
  height: 30px;
}
.t-btn.play svg {
  width: 25px;
  height: 25px;
}
.badge {
  position: absolute;
  top: 0;
  right: -1px;
  font-size: 8px;
  font-weight: 700;
  color: var(--accent);
}

.np-pill {
  position: relative;
  display: flex;
  align-items: center;
  gap: 9px;
  width: 300px;
  max-width: 34vw;
  height: 36px;
  padding: 0 10px;
  border-radius: 7px;
  background: var(--pill-bg);
  border: 1px solid var(--hairline);
  overflow: hidden;
  text-align: left;
  transition: background 0.25s var(--ease-out-soft), transform 0.35s var(--ease-spring);
}
.np-pill:hover:not(:disabled) {
  background: var(--hover);
}
.np-pill:active:not(:disabled) {
  transform: scale(0.98);
  transition-duration: 0.09s;
}
.np-pill:disabled {
  cursor: default;
}
.p-cover {
  width: 26px;
  height: 26px;
  border-radius: 4px;
  overflow: hidden;
  flex: none;
  background: var(--bg-3);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.2);
}
.p-cover img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}
.p-cover .ph {
  width: 100%;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 13px;
  color: var(--text-3);
}
.p-meta {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}
.p-title {
  font-size: 11.5px;
  font-weight: 600;
  color: var(--text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.p-title.muted {
  color: var(--text-3);
  font-weight: 500;
}
.p-artist {
  font-size: 10px;
  color: var(--text-2);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.p-line {
  position: absolute;
  left: 0;
  bottom: 0;
  height: 2px;
  background: var(--accent);
  border-radius: 0 1px 0 0;
  transition: width 0.25s linear;
}

.right {
  display: flex;
  align-items: center;
  gap: 10px;
  flex: none;
}
.vol-ico {
  width: 16px;
  height: 16px;
  color: var(--text-2);
}
.vol {
  width: 84px;
}

.caps {
  display: flex;
  align-self: stretch;
  flex: none;
}
.cap {
  width: 46px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-2);
  border-radius: 0;
  transition: background 0.18s ease, color 0.18s ease;
}
.cap svg {
  width: 15px;
  height: 15px;
}
.cap:hover {
  background: var(--hover);
  color: var(--text);
}
.cap:active {
  transform: none;
}
.cap.close:hover {
  background: var(--danger);
  color: #fff;
}
</style>
