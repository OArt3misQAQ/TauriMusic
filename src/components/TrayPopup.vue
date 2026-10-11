<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { convertFileSrc } from "@tauri-apps/api/core";

/** 托盘右键的迷你播放面板(独立 WebView 窗口):
 *  播放状态由主窗口经 media://state 事件广播,按钮动作 emit 回主窗口执行;
 *  打开时先发 state-request 请求一次当前状态,避免显示旧数据 */

interface TrayState {
  title: string;
  artist: string;
  cover: string | null;
  playing: boolean;
  position: number;
  duration: number;
}

const state = ref<TrayState>({
  title: "",
  artist: "",
  cover: null,
  playing: false,
  position: 0,
  duration: 0,
});

const coverUrl = computed(() => (state.value.cover ? convertFileSrc(state.value.cover) : null));
const pct = computed(() =>
  state.value.duration > 0 ? (state.value.position / state.value.duration) * 100 : 0,
);
const timeText = computed(
  () => `${fmt(state.value.position)} / ${fmt(state.value.duration)}`,
);

function fmt(s: number): string {
  const v = Number.isFinite(s) && s > 0 ? Math.floor(s) : 0;
  return `${Math.floor(v / 60)}:${String(v % 60).padStart(2, "0")}`;
}

function act(a: string): void {
  void emit("media://action", a);
}
function sys(a: string): void {
  void emit("media://sys", a);
}
function onKey(e: KeyboardEvent): void {
  if (e.key === "Escape") void getCurrentWindow().hide();
}

let cleanups: (() => void)[] = [];

onMounted(async () => {
  cleanups.push(
    await listen<TrayState>("media://state", (e) => {
      state.value = e.payload;
    }),
  );
  // 主窗口收到请求后重发一次当前状态
  await emit("media://state-request");
  window.addEventListener("keydown", onKey);
});
onUnmounted(() => {
  cleanups.forEach((fn) => fn());
  cleanups = [];
  window.removeEventListener("keydown", onKey);
});
</script>

<template>
  <div class="card">
    <div class="main">
      <div class="cover">
        <img v-if="coverUrl" :src="coverUrl" alt="" />
        <span v-else class="ph">♪</span>
      </div>
      <div class="meta">
        <div class="title" :title="state.title">{{ state.title || "未在播放" }}</div>
        <div class="artist" :title="state.artist">{{ state.artist }}</div>
        <div class="bar">
          <div class="fill" :style="{ width: pct + '%' }"></div>
        </div>
        <div class="time">{{ timeText }}</div>
      </div>
    </div>
    <div class="controls">
      <button class="ctl" title="上一曲" @click="act('prev')">
        <svg viewBox="0 0 24 24">
          <path d="M7 5.8v12.4" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" />
          <path d="M18.5 6.3v11.4L9.4 12z" fill="currentColor" />
        </svg>
      </button>
      <button class="ctl play" :title="state.playing ? '暂停' : '播放'" @click="act('toggle')">
        <svg v-if="state.playing" viewBox="0 0 24 24">
          <path d="M7.6 5.5h3.3v13H7.6zM13.2 5.5h3.3v13h-3.3z" fill="currentColor" />
        </svg>
        <svg v-else viewBox="0 0 24 24">
          <path d="M8.2 5.5v13L19 12z" fill="currentColor" />
        </svg>
      </button>
      <button class="ctl" title="下一曲" @click="act('next')">
        <svg viewBox="0 0 24 24">
          <path d="M17 5.8v12.4" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" />
          <path d="M5.5 6.3v11.4L14.6 12z" fill="currentColor" />
        </svg>
      </button>
      <div class="spacer"></div>
      <button class="ghost" title="显示主窗口" @click="sys('show')">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round">
          <rect x="4.5" y="5.5" width="15" height="13" rx="2" />
          <path d="M4.5 9.5h15" />
        </svg>
      </button>
      <button class="ghost quit" title="退出 TauriMusic" @click="sys('quit')">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round">
          <path d="M6 6l12 12M18 6L6 18" />
        </svg>
      </button>
    </div>
  </div>
</template>

<style scoped>
/* 弹窗窗口本体透明,卡片自带圆角与投影;背景/皮肤令牌由共享的 localStorage 决定 */
:global(html),
:global(body),
:global(#app) {
  background: transparent !important;
  overflow: hidden;
}
:global(body) {
  margin: 0;
}

.card {
  margin: 6px;
  height: calc(100% - 12px);
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 14px;
  border-radius: 14px;
  background: var(--bg-2);
  border: 1px solid var(--hairline);
  box-shadow: 0 14px 44px rgba(0, 0, 0, 0.4), 0 2px 8px rgba(0, 0, 0, 0.22);
  user-select: none;
}

.main {
  display: flex;
  gap: 12px;
  min-width: 0;
}
.cover {
  width: 56px;
  height: 56px;
  flex: none;
  border-radius: 8px;
  overflow: hidden;
  background: var(--pill-bg);
  display: flex;
  align-items: center;
  justify-content: center;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.25);
}
.cover img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}
.cover .ph {
  font-size: 22px;
  color: var(--text-3);
}
.meta {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
  justify-content: center;
}
.title {
  font-size: 13.5px;
  font-weight: 700;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.artist {
  font-size: 11.5px;
  color: var(--text-2);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.bar {
  height: 4px;
  border-radius: 999px;
  background: var(--pill-bg);
  overflow: hidden;
  margin-top: 3px;
}
.fill {
  height: 100%;
  border-radius: inherit;
  background: var(--accent);
  transition: width 0.3s linear;
}
.time {
  font-size: 10px;
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
}

.controls {
  display: flex;
  align-items: center;
  gap: 8px;
}
.controls .spacer {
  flex: 1;
}
.ctl {
  width: 34px;
  height: 34px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-2);
  transition: background 0.18s ease, color 0.18s ease, transform 0.25s var(--ease-spring);
}
.ctl svg {
  width: 17px;
  height: 17px;
}
.ctl:hover {
  background: var(--hover);
  color: var(--text);
}
.ctl:active {
  transform: scale(0.9);
}
.ctl.play {
  width: 40px;
  height: 40px;
  background: var(--accent);
  color: #fff;
  box-shadow: 0 3px 10px color-mix(in srgb, var(--accent) 45%, transparent);
}
.ctl.play svg {
  width: 19px;
  height: 19px;
}
.ctl.play:hover {
  background: var(--accent-hover);
  color: #fff;
}
.ghost {
  width: 28px;
  height: 28px;
  border-radius: 7px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-3);
  transition: background 0.18s ease, color 0.18s ease;
}
.ghost svg {
  width: 14px;
  height: 14px;
}
.ghost:hover {
  background: var(--hover);
  color: var(--text);
}
.ghost.quit:hover {
  background: var(--danger-bg);
  color: var(--danger);
}
</style>
