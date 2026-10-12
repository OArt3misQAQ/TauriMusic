<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { convertFileSrc } from "@tauri-apps/api/core";

/** 托盘右键的迷你播放面板(独立 WebView 窗口):
 *  播放状态由主窗口经 media://state 事件广播,按钮动作 emit 回主窗口执行;
 *  打开时先发 state-request 请求一次当前状态,避免显示旧数据。
 *  窗口即面板:WebView 透明在 Windows 上透不出去(runtime 黑色软缓冲层),
 *  所以面板直接铺满窗口,圆角与投影交给系统 DWM。
 *  视觉概念「迷你黑胶唱机」:封面做成旋转唱片,与 logo「墨夜胶片」同一语言 */

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

/* 进度可拖:拖动中用本地值预览(主窗口的状态广播约 1Hz,不屏蔽会把滑块拽回去),
   松手才提交 seek —— "seek:<秒>" 动作由主窗口的 media 桥解析 */
const dragging = ref(false);
const dragPos = ref(0);
const barPos = computed(() => (dragging.value ? dragPos.value : state.value.position));
const barPct = computed(() =>
  state.value.duration > 0 ? (barPos.value / state.value.duration) * 100 : 0,
);
const timeText = computed(() => `${fmt(barPos.value)} / ${fmt(state.value.duration)}`);

function onDrag(e: Event): void {
  dragging.value = true;
  dragPos.value = Number((e.target as HTMLInputElement).value);
}
function onSeekCommit(e: Event): void {
  const v = Number((e.target as HTMLInputElement).value);
  dragging.value = false;
  if (state.value.duration > 0) void emit("media://action", `seek:${v}`);
}

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
function onWinBlur(): void {
  dragging.value = false;
}
function requestState(): void {
  void emit("media://state-request");
}

let cleanups: (() => void)[] = [];

onMounted(async () => {
  cleanups.push(
    await listen<TrayState>("media://state", (e) => {
      state.value = e.payload;
    }),
  );
  // 主窗口收到请求后重发一次当前状态
  requestState();
  window.addEventListener("keydown", onKey);
  // 面板失焦即被隐藏:拖到一半丢了焦点要把拖拽态复位,避免下次打开滑块卡死
  window.addEventListener("blur", onWinBlur);
  // 注意:状态重拉的权威时机在 Rust 侧 show_tray_popup(app.emit 强制主窗口
  // 广播)——JS 的 focus/visibilitychange 补偿受 Win32 前台锁与 WebView2
  // 可见性语义限制不可靠(visibilitychange 甚至永不触发),不要依赖
});
onUnmounted(() => {
  cleanups.forEach((fn) => fn());
  cleanups = [];
  window.removeEventListener("keydown", onKey);
  window.removeEventListener("blur", onWinBlur);
});
</script>

<template>
  <div class="tray-card">
    <!-- 铭牌行:器材丝印风格的小字距大写,右侧呼吸灯只在播放时亮起 -->
    <header class="t-head">
      <span class="t-logo">
        <svg viewBox="0 0 24 24">
          <circle cx="12" cy="12" r="8.6" fill="none" stroke="#fff" stroke-width="2" />
          <circle cx="12" cy="12" r="3.4" fill="#fff" />
          <circle cx="12" cy="12" r="1.2" style="fill: var(--accent)" />
        </svg>
      </span>
      <span class="t-name">TauriMusic</span>
      <Transition name="lamp">
        <span v-if="state.playing" class="t-live" title="正在播放"></span>
      </Transition>
    </header>
    <div class="t-rule" aria-hidden="true"></div>

    <!-- 唱机区:旋转黑胶(封面即唱片标签) + 曲目信息 -->
    <div class="main">
      <div class="vinyl" :class="{ spinning: state.playing && coverUrl }">
        <div class="disc">
          <img v-if="coverUrl" :src="coverUrl" alt="" />
          <div v-else class="noph">♪</div>
        </div>
        <span class="spindle" aria-hidden="true"></span>
      </div>
      <div class="meta">
        <Transition name="meta" mode="out-in">
          <div :key="state.title + state.artist" class="meta-body">
            <div class="title" :title="state.title">{{ state.title || "未在播放" }}</div>
            <div class="artist" :title="state.artist">{{ state.artist || "—" }}</div>
          </div>
        </Transition>
      </div>
    </div>

    <!-- 进度:可拖的唱针轨——白点即唱针,拖动中时间实时预览,松手跳转 -->
    <div class="progress">
      <input
        class="seekrange"
        type="range"
        min="0"
        :max="state.duration || 1"
        step="0.1"
        :value="barPos"
        :disabled="!state.duration"
        :style="{ '--fill': barPct + '%' }"
        @input="onDrag"
        @change="onSeekCommit"
      />
      <span class="time">{{ timeText }}</span>
    </div>

    <div class="controls">
      <button class="ctl" title="上一曲" @click="act('prev')">
        <svg viewBox="0 0 24 24" fill="currentColor">
          <path d="M11.4 6.4v11.2L3.6 12z" />
          <path d="M20 6.4v11.2L12.2 12z" />
        </svg>
      </button>
      <button class="ctl play" :title="state.playing ? '暂停' : '播放'" @click="act('toggle')">
        <svg v-if="state.playing" viewBox="0 0 24 24" fill="currentColor">
          <rect x="7.2" y="5.5" width="3.4" height="13" rx="1.2" />
          <rect x="13.4" y="5.5" width="3.4" height="13" rx="1.2" />
        </svg>
        <svg v-else viewBox="0 0 24 24" fill="currentColor">
          <path d="M8.2 5.5v13L19 12z" />
        </svg>
      </button>
      <button class="ctl" title="下一曲" @click="act('next')">
        <svg viewBox="0 0 24 24" fill="currentColor">
          <path d="M12.6 6.4v11.2L20.4 12z" />
          <path d="M4 6.4v11.2L11.8 12z" />
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
/* 窗口即面板:面板铺满窗口,圆角交给系统 DWM;
   三套皮肤材质(玻璃浅/深、纸质)在 style.css 按类名接管 */
.tray-card {
  height: 100%;
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
  padding: 0 14px 12px;
  background: var(--bg-2);
  user-select: none;
}

/* ===== 铭牌行 ===== */
.t-head {
  height: 40px;
  flex: none;
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 0 2px;
}
.t-logo {
  width: 20px;
  height: 20px;
  border-radius: 6px;
  background: var(--accent);
  display: flex;
  align-items: center;
  justify-content: center;
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.25);
}
.t-logo svg {
  width: 13px;
  height: 13px;
}
.t-name {
  /* 与主窗口顶栏同款品牌字:自然大小写、中等字重、微字距——
     Apple 式的品牌呈现从不靠全大写和粗黑撑场面 */
  font-size: 12.5px;
  font-weight: 600;
  letter-spacing: 0.02em;
  color: var(--text);
}
.t-live {
  margin-left: auto;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--accent);
  box-shadow: 0 0 8px var(--accent);
  animation: live 1.6s ease-in-out infinite;
}
@keyframes live {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.25;
  }
}
.lamp-enter-active {
  transition: opacity 0.3s ease;
}
.lamp-leave-active {
  transition: opacity 0.15s ease;
}
.lamp-enter-from,
.lamp-leave-to {
  opacity: 0;
}
.t-rule {
  height: 1px;
  flex: none;
  margin: 0 -14px;
  background: rgba(255, 255, 255, 0.8);
}

/* ===== 唱机区 ===== */
.main {
  flex: 1;
  min-height: 0;
  display: flex;
  align-items: center;
  gap: 13px;
}
/* 黑胶:外圈盘体 + 放射纹路 + 固定光泽,封面是旋转的标签,中心是唱针孔 */
.vinyl {
  width: 64px;
  height: 64px;
  flex: none;
  position: relative;
  border-radius: 50%;
  background:
    /* 唱片纹路:从孔缘向外的一圈圈细沟 */
    repeating-radial-gradient(circle at 50% 50%,
      rgba(255, 255, 255, 0.055) 0 1px,
      transparent 1px 3px),
    #131313;
  box-shadow:
    inset 0 0 0 1px rgba(255, 255, 255, 0.08),
    0 3px 10px rgba(0, 0, 0, 0.35);
}
/* 固定光泽:光源不动,只有标签在转——唱片的"反光感" */
.vinyl::before {
  content: "";
  position: absolute;
  inset: 0;
  border-radius: 50%;
  background: conic-gradient(from 210deg,
      transparent 0deg,
      rgba(255, 255, 255, 0.1) 18deg,
      transparent 42deg,
      transparent 178deg,
      rgba(255, 255, 255, 0.07) 198deg,
      transparent 224deg);
  pointer-events: none;
}
.disc {
  position: absolute;
  inset: 7px;
  border-radius: 50%;
  overflow: hidden;
  background: #1c1c1e;
  box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.06);
  animation: spin 3.2s linear infinite;
  animation-play-state: paused;
}
.vinyl.spinning .disc {
  animation-play-state: running;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
.disc img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}
.disc .noph {
  width: 100%;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 18px;
  color: rgba(255, 255, 255, 0.3);
}
/* 唱针孔:固定不转,中心一点 accent 唱轴 */
.spindle {
  position: absolute;
  left: 50%;
  top: 50%;
  width: 12px;
  height: 12px;
  transform: translate(-50%, -50%);
  border-radius: 50%;
  background: var(--bg-2);
  box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.3);
}
.spindle::after {
  content: "";
  position: absolute;
  inset: 4px;
  border-radius: 50%;
  background: var(--accent);
}

.meta {
  flex: 1;
  min-width: 0;
}
.meta-body {
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.meta-enter-active {
  transition: opacity 0.22s ease, transform 0.26s var(--ease-out-soft);
}
.meta-leave-active {
  transition: opacity 0.14s ease, transform 0.14s ease;
}
.meta-enter-from {
  opacity: 0;
  transform: translateY(6px);
}
.meta-leave-to {
  opacity: 0;
  transform: translateY(-6px);
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

/* ===== 进度:可拖的唱针轨 ===== */
.progress {
  flex: none;
  display: flex;
  align-items: center;
  gap: 10px;
  margin: 4px 0 8px;
}
.seekrange {
  flex: 1;
  height: 3px;
  border-radius: 999px;
  background: linear-gradient(
    to right,
    var(--accent) var(--fill, 0%),
    var(--pill-bg) var(--fill, 0%)
  );
}
/* 滑块即唱针:白点带 accent 光晕,悬停略微放大 */
.seekrange::-webkit-slider-thumb {
  -webkit-appearance: none;
  appearance: none;
  width: 9px;
  height: 9px;
  border-radius: 50%;
  background: #fff;
  box-shadow:
    0 0 0 1px rgba(0, 0, 0, 0.12),
    0 0 8px var(--accent);
  transition: transform 0.15s ease;
}
.seekrange:hover::-webkit-slider-thumb {
  transform: scale(1.25);
}
.seekrange:disabled {
  cursor: default;
}
.time {
  flex: none;
  font-size: 10px;
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
}

/* ===== 控制行:双三角步进与主窗口播放页同语言,播放键红色大圆 ===== */
.controls {
  flex: none;
  display: flex;
  align-items: center;
  gap: 8px;
}
.controls .spacer {
  flex: 1;
}
.ctl {
  width: 32px;
  height: 32px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-2);
  transition: background 0.18s ease, color 0.18s ease, transform 0.25s var(--ease-spring);
}
.ctl svg {
  width: 15px;
  height: 15px;
}
.ctl:hover {
  background: var(--hover);
  color: var(--text);
}
.ctl:active {
  transform: scale(0.88);
}
.ctl.play {
  width: 40px;
  height: 40px;
  background: var(--accent);
  color: #fff;
  box-shadow: 0 3px 12px color-mix(in srgb, var(--accent) 45%, transparent);
}
.ctl.play svg {
  width: 18px;
  height: 18px;
}
.ctl.play:hover {
  background: var(--accent-hover);
  color: #fff;
  transform: scale(1.06);
}
.ctl.play:active {
  transform: scale(0.94);
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
