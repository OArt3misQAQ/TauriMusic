<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import { api, type LyricLine } from "../api";
import {
  player,
  current,
  toggle,
  next,
  prev,
  seek,
  fmtTime,
  upcomingTracks,
  jumpToQueueIndex,
  removeFromQueue,
} from "../stores/player";
import ModeIcon from "./ModeIcon.vue";
import { ui } from "../stores/ui";

const coverUrl = computed(() => {
  // 全屏播放页是大图消费方,优先 512px 档;旧记录没有大图时退回小图
  const src = current.value?.coverLarge ?? current.value?.cover;
  return src ? convertFileSrc(src) : null;
});
const bgStyle = computed(() => ({
  "--np-cover": coverUrl.value ? `url("${coverUrl.value}")` : "none",
}));
const progress = computed(() =>
  player.duration > 0 ? (seekShow.value / player.duration) * 100 : 0,
);

/* 拖动进度条:拖动中本地预览,松手(change)才真正 seek——
   input 每 tick 都写 audio.currentTime 会造成音频抖动 */
const seeking = ref<number | null>(null);
const seekShow = computed(() => seeking.value ?? player.position);
function onSeekInput(e: Event): void {
  seeking.value = Number((e.target as HTMLInputElement).value);
}
function onSeekCommit(e: Event): void {
  seek(Number((e.target as HTMLInputElement).value));
  seeking.value = null;
}

const synced = ref<LyricLine[] | null>(null);
const plain = ref<string | null>(null);
const activeIdx = ref(-1);
const lyricsEl = ref<HTMLElement | null>(null);
const noLyrics = ref(false);
const loadingLyrics = ref(false);

watch(current, loadLyrics, { immediate: true });

async function loadLyrics(): Promise<void> {
  synced.value = null;
  plain.value = null;
  activeIdx.value = -1;
  noLyrics.value = false;
  loadingLyrics.value = true;
  const track = current.value;
  if (!track) {
    loadingLyrics.value = false;
    return;
  }
  try {
    const payload = await api.getLyrics(track.id);
    if (current.value?.id !== track.id) return;
    if (!payload) {
      noLyrics.value = true;
      return;
    }
    synced.value = payload.synced;
    plain.value = payload.plain;
  } catch (err) {
    console.error("歌词加载失败", err);
    if (current.value?.id !== track.id) return;
    noLyrics.value = true;
  } finally {
    // 快速切歌时先返回的旧请求不能掐掉新请求的"加载中"指示
    if (current.value?.id === track.id) loadingLyrics.value = false;
  }
}

watch(
  () => player.position,
  (pos) => {
    const lines = synced.value;
    if (!lines || lines.length === 0) return;
    const t = pos * 1000 + 200;
    // 二分查找当前行(线性扫描在长歌词下每秒多次全量遍历)
    let lo = 0;
    let hi = lines.length - 1;
    let idx = -1;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      if (lines[mid].timeMs <= t) {
        idx = mid;
        lo = mid + 1;
      } else {
        hi = mid - 1;
      }
    }
    if (idx !== activeIdx.value) {
      activeIdx.value = idx;
      if (idx >= 0) scrollToLine(idx);
    }
  },
);

function scrollToLine(idx: number): void {
  const el = lyricsEl.value;
  if (!el) return;
  const child = el.children[idx] as HTMLElement | undefined;
  if (!child) return;
  el.scrollTo({
    top: child.offsetTop - el.clientHeight / 2 + child.clientHeight / 2,
    behavior: "smooth",
  });
}

function close(): void {
  ui.nowPlayingOpen = false;
}

/* 播放队列抽屉:展示本轮循环的真实播放顺序(随机模式即洗牌袋剩余顺序)。
   computed 依赖 queue/index/mode,切歌/移除后自动刷新 */
const queueOpen = ref(false);
const queueList = computed(() => (queueOpen.value ? upcomingTracks() : []));

/* 播放页收起时顺带收起队列,下次打开是干净状态 */
watch(
  () => ui.nowPlayingOpen,
  (open) => {
    if (!open) queueOpen.value = false;
  },
);
</script>

<template>
  <!-- queue-open:队列抽屉打开时,右上角收起箭头让位到抽屉左侧 -->
  <div class="np" :class="{ 'queue-open': queueOpen }" :style="bgStyle">
    <button class="close" title="关闭" @click="close">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="m6 9.2 6 6 6-6" />
      </svg>
    </button>

    <div class="stage">
      <div class="art">
        <img v-if="coverUrl" :src="coverUrl" alt="" />
        <div v-else class="ph">♪</div>
      </div>

      <div class="info">
        <div class="title">{{ current?.title }}</div>
        <div class="artist">{{ current?.artist }}{{ current?.album ? " — " + current.album : "" }}</div>
      </div>

      <div class="lyrics-wrap">
        <div v-if="synced && synced.length" ref="lyricsEl" class="lyrics synced">
          <p
            v-for="(line, i) in synced"
            :key="i"
            :class="{ active: i === activeIdx, near: Math.abs(i - activeIdx) === 1, far: Math.abs(i - activeIdx) > 1 }"
            @click="seek(line.timeMs / 1000)"
          >
            {{ line.text || "♪" }}<span v-if="line.translation" class="trans">{{ line.translation }}</span>
          </p>
        </div>
        <div v-else-if="plain" class="lyrics plain">
          <p v-for="(line, i) in plain.split('\n')" :key="i">{{ line }}</p>
        </div>
        <div v-else-if="loadingLyrics" class="lyrics none">歌词加载中…</div>
        <div v-else class="lyrics none">暂无歌词</div>
      </div>
    </div>

    <div class="bottom">
      <!-- 三区一行:模式键回到播放控制旁(左),控制组居中,音量在右 -->
      <div class="controls">
        <div class="zone left">
          <ModeIcon class="t-btn" />
        </div>
        <div class="zone mid">
          <!-- 传输三键按 Apple Music 全屏播放器语言重绘:双三角步进曲 + 实心圆播放键 -->
          <button class="t-btn" title="上一曲" @click="prev()">
            <svg viewBox="0 0 24 24" fill="currentColor">
              <path d="M11.4 6.4v11.2L3.6 12z" />
              <path d="M20 6.4v11.2L12.2 12z" />
            </svg>
          </button>
          <button class="t-btn play" :title="player.playing ? '暂停' : '播放'" @click="toggle()">
            <!-- 播放标本身即设计:圆角三角/双粗圆角条,比步进曲大一整档,不套任何底盘 -->
            <svg v-if="!player.playing" viewBox="0 0 24 24">
              <path d="M8 5.4v13.2L19.2 12z" fill="currentColor" stroke="currentColor" stroke-width="2.2" stroke-linejoin="round" />
            </svg>
            <svg v-else viewBox="0 0 24 24" fill="currentColor">
              <rect x="6.6" y="4.8" width="4" height="14.4" rx="1.5" />
              <rect x="13.4" y="4.8" width="4" height="14.4" rx="1.5" />
            </svg>
          </button>
          <button class="t-btn" title="下一曲" @click="next()">
            <svg viewBox="0 0 24 24" fill="currentColor">
              <path d="M12.6 6.4v11.2L20.4 12z" />
              <path d="M4 6.4v11.2L11.8 12z" />
            </svg>
          </button>
        </div>
        <div class="zone right">
          <!-- 音量只保留顶栏一处(与传输控件同属全局),播放页底部不再重复 -->
          <button class="t-btn" :class="{ on: queueOpen }" title="播放队列" @click="queueOpen = !queueOpen">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round">
              <path d="M4 6h16M4 12h16M4 18h9" />
            </svg>
          </button>
        </div>
      </div>
      <div class="seek">
        <span class="time">{{ fmtTime(seekShow) }}</span>
        <input
          class="range"
          type="range"
          min="0"
          :max="player.duration || 1"
          step="0.1"
          :value="seekShow"
          :style="{ '--fill': progress + '%' }"
          @input="onSeekInput"
          @change="onSeekCommit"
        />
        <span class="time">{{ fmtTime(player.duration) }}</span>
      </div>
    </div>

    <!-- 播放队列抽屉:本轮循环的真实播放顺序;点行切歌,悬停移除(不删文件) -->
    <Transition name="drawer">
      <aside v-if="queueOpen" class="queue">
        <header class="q-head">
          <span class="q-title">播放队列<span class="q-count">{{ queueList.length + (current ? 1 : 0) }} 首</span></span>
          <button class="q-close" title="收起" @click="queueOpen = false">
            <svg viewBox="0 0 24 24"><path d="m6 6 12 12M18 6 6 18" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" /></svg>
          </button>
        </header>
        <div class="q-body">
          <div v-if="current" class="q-row now" :class="{ paused: !player.playing }">
            <span class="q-eq" aria-hidden="true"><i></i><i></i><i></i></span>
            <span class="q-meta">
              <span class="q-t">{{ current.title }}</span>
              <span class="q-a">{{ current.artist }}</span>
            </span>
            <span class="q-dur">{{ fmtTime(player.duration) }}</span>
          </div>
          <div class="q-sep">接下来</div>
          <TransitionGroup name="qlist" tag="div" class="q-list">
            <div
              v-for="(item, i) in queueList"
              :key="item.queueIndex"
              class="q-row"
              @click="jumpToQueueIndex(item.queueIndex)"
            >
              <span class="q-idx">{{ i + 1 }}</span>
              <span class="q-meta">
                <span class="q-t">{{ item.track.title }}</span>
                <span class="q-a">{{ item.track.artist }}</span>
              </span>
              <span class="q-dur">{{ fmtTime(item.track.duration) }}</span>
              <button class="q-remove" title="从队列移除" @click.stop="removeFromQueue(item.queueIndex)">
                <svg viewBox="0 0 24 24"><path d="m6 6 12 12M18 6 6 18" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" /></svg>
              </button>
            </div>
          </TransitionGroup>
          <div v-if="!queueList.length" class="q-empty">{{ current ? "本轮播放到这里就结束啦" : "队列还是空的" }}</div>
        </div>
      </aside>
    </Transition>
  </div>
</template>

<style scoped>
.np {
  /* 播放页局部令牌:纸质皮肤在 style.css 末尾整组覆写(印刷节目单)。
     底色压到墨级——播放页是全应用最大的色块,灰半档全应用就"黑不正" */
  --np-bg: #1d1e20;
  --np-fg: #ffffff;
  --np-fg-2: rgba(255, 255, 255, 0.62);
  --np-fg-3: rgba(255, 255, 255, 0.42);
  --np-fg-soft: rgba(255, 255, 255, 0.75);
  --np-fg-strong: rgba(255, 255, 255, 0.9);
  --np-fg-mute: rgba(255, 255, 255, 0.55);
  --np-plain: rgba(255, 255, 255, 0.8);
  --np-btn-bg: rgba(255, 255, 255, 0.18);
  --np-btn-bg-hover: rgba(255, 255, 255, 0.3);
  --np-close-bg: rgba(255, 255, 255, 0.14);
  --np-close-hover: rgba(255, 255, 255, 0.26);
  --np-close-fg: rgba(255, 255, 255, 0.85);
  --np-track: rgba(255, 255, 255, 0.28);
  --np-thumb: #ffffff;
  --np-art-shadow: 0 18px 60px rgba(0, 0, 0, 0.55);
  --np-art-bg: rgba(255, 255, 255, 0.08);
  --np-text-shadow: 0 1px 8px rgba(0, 0, 0, 0.4);
  position: absolute;
  inset: 0;
  z-index: 60;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  color: var(--np-fg);
  background: var(--np-bg);
  /* 顶栏悬浮在最上层(z-100):为它预留 48px,标题/封面/歌词不再钻到顶栏底下 */
  padding-top: 48px;
}
.np::before {
  content: "";
  position: absolute;
  inset: -90px;
  background-image: var(--np-cover);
  background-size: cover;
  background-position: center;
  filter: blur(80px) saturate(1.5) brightness(0.5);
  transform: scale(1.1);
}

.close {
  position: absolute;
  top: 60px;
  right: 18px;
  z-index: 5;
  width: 34px;
  height: 34px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--np-close-fg);
  background: var(--np-close-bg);
  backdrop-filter: blur(10px);
  transition:
    right 0.38s var(--ease-out-soft),
    background 0.25s var(--ease-out-soft),
    transform 0.35s var(--ease-spring);
}
/* 队列抽屉打开:收起箭头平滑让位到抽屉左侧(宽度与抽屉一致) */
.np.queue-open .close {
  right: calc(min(320px, 46%) + 18px);
}
.close:hover {
  background: var(--np-close-hover);
}
.close svg {
  width: 18px;
  height: 18px;
}

.stage {
  position: relative;
  flex: 1;
  display: grid;
  grid-template-columns: minmax(260px, 380px) 1fr;
  gap: 40px;
  align-items: center;
  padding: 48px 56px 12px;
  min-height: 0;
}

.art {
  position: relative;
  aspect-ratio: 1;
  max-height: 100%;
  border-radius: 12px;
  overflow: hidden;
  box-shadow: var(--np-art-shadow);
  background: var(--np-art-bg);
}
.art img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}
.art .ph {
  width: 100%;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 64px;
  color: var(--np-fg-3);
}

.info {
  position: absolute;
  top: 26px;
  left: 56px;
  max-width: 420px;
  text-shadow: var(--np-text-shadow);
}
.info .title {
  font-size: 22px;
  font-weight: 700;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.info .artist {
  font-size: 14px;
  opacity: 0.82;
  margin-top: 4px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.lyrics-wrap {
  height: 100%;
  min-height: 0;
  display: flex;
  align-items: stretch;
  mask-image: linear-gradient(to bottom, transparent, #000 12%, #000 86%, transparent);
}
.lyrics {
  flex: 1;
  overflow-y: auto;
  padding: 42% 8px 42%;
  scrollbar-width: none;
}
.lyrics::-webkit-scrollbar {
  display: none;
}
.lyrics.synced p {
  font-size: 22px;
  font-weight: 700;
  line-height: 1.45;
  padding: 7px 0;
  color: var(--np-fg-3);
  cursor: pointer;
  transition: color 0.4s var(--ease-out-soft), transform 0.4s var(--ease-out-soft);
  transform-origin: left center;
}
/* 对照译文:小一号、轻一档,随所在行一起被高亮/缩放;pre-line 兼容多段译文的换行 */
.lyrics.synced p .trans {
  display: block;
  font-size: 15px;
  font-weight: 500;
  line-height: 1.5;
  opacity: 0.78;
  white-space: pre-line;
}
.lyrics.synced p.near {
  color: var(--np-fg-2);
}
.lyrics.synced p.active {
  color: var(--np-fg);
  transform: scale(1.02);
}
.lyrics.synced p:hover {
  color: var(--np-fg-strong);
}
.lyrics.plain p {
  font-size: 16px;
  line-height: 1.9;
  color: var(--np-plain);
  white-space: pre-wrap;
}
.lyrics.none {
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 15px;
  color: var(--np-fg-mute);
}

.bottom {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  padding: 10px 0 26px;
}
/* 控制行:与进度条同宽同列——左(模式键)/中(播放控制)/右(队列)三区分布,
   不再被 stretch 拉到窗口两角;窄窗口时左右区让位给中间的控制组 */
.controls {
  display: grid;
  grid-template-columns: 1fr auto 1fr;
  align-items: center;
  width: 560px;
  max-width: 90vw;
}
.zone.left {
  justify-self: start;
}
.zone.mid {
  display: flex;
  align-items: center;
  gap: 26px;
}
.zone.right {
  justify-self: end;
}
.t-btn {
  width: 28px;
  height: 28px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--np-fg-soft);
  transition: color 0.15s ease, transform 0.3s var(--ease-spring);
}
.t-btn:hover {
  color: var(--np-fg);
}
.t-btn:active {
  transform: scale(0.86);
  transition-duration: 0.08s;
}
.t-btn.on {
  color: var(--np-fg);
}
.t-btn svg {
  width: 21px;
  height: 21px;
}
.zone.mid .t-btn svg {
  width: 26px;
  height: 26px;
}
/* 播放标必须比上面通配更大一档:这里用 .zone.mid 前缀压过通配的特异性,
   否则 22px 会盖掉大标尺寸——上次"看不出大"就是这个原因 */
.zone.mid .t-btn.play svg {
  width: 42px;
  height: 42px;
}
/* 播放键:超级大标——不套圆盘,靠与步进曲一整档的尺寸差当主角;
   参考做法里暂停就是两根粗圆角条,播放是一颗圆角大三角 */
.t-btn.play {
  width: 60px;
  height: 50px;
}
.t-btn.play:hover {
  transform: scale(1.08);
}
.t-btn.play:active {
  transform: scale(0.95);
}
.t-btn.play svg {
  width: 38px;
  height: 38px;
}

.seek {
  display: flex;
  align-items: center;
  gap: 12px;
  width: 560px;
  max-width: 90vw;
}
.time {
  font-size: 11px;
  color: var(--np-fg-soft);
  font-variant-numeric: tabular-nums;
  min-width: 36px;
}
.time:last-child {
  text-align: right;
}
.seek .range {
  flex: 1;
  background: linear-gradient(to right, var(--np-fg) var(--fill, 0%), var(--np-track) var(--fill, 0%));
}
.seek .range::-webkit-slider-thumb {
  background: var(--np-thumb);
}

/* ===== 播放队列抽屉:右侧滑入的本轮循环面板 =====
   材质跟随播放页:玻璃下是磨纱暗抽屉,纸质下由末尾的印刷插页规则接管 */
.queue {
  position: absolute;
  top: 0;
  right: 0;
  bottom: 0;
  z-index: 6; /* 高于右上角的收起按钮,抽屉自带关闭键 */
  width: 320px;
  max-width: 46%;
  display: flex;
  flex-direction: column;
  background: rgba(13, 14, 17, 0.66);
  backdrop-filter: blur(30px) saturate(1.4);
  -webkit-backdrop-filter: blur(30px) saturate(1.4);
  border-left: 1px solid rgba(255, 255, 255, 0.09);
}
.q-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 18px 18px 12px;
}
.q-title {
  font-size: 14px;
  font-weight: 700;
  color: var(--np-fg);
}
.q-count {
  margin-left: 8px;
  font-size: 11px;
  font-weight: 500;
  color: var(--np-fg-3);
}
.q-close {
  width: 26px;
  height: 26px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--np-fg-2);
  transition: background 0.2s ease, color 0.2s ease;
}
.q-close:hover {
  background: var(--np-btn-bg);
  color: var(--np-fg);
}
.q-close svg {
  width: 13px;
  height: 13px;
}
/* 抽屉自持一面光斑:它的 z 层盖过播放页扫光,不自己发光就是一块"照不亮的死玻璃";
   z -1 让光落在抽屉底、行列表之下,与侧栏的镜面高光同一套语言 */
html.glass .np .queue::after {
  content: "";
  position: absolute;
  inset: 0;
  z-index: -1;
  pointer-events: none;
  mix-blend-mode: screen;
  background: radial-gradient(
    420px circle at var(--pointer-x, -999px) var(--pointer-y, -999px),
    rgba(255, 255, 255, 0.09),
    rgba(255, 255, 255, 0) 65%
  );
}
.q-body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 0 10px 16px;
  scrollbar-width: none;
}
.q-body::-webkit-scrollbar {
  display: none;
}
.q-list {
  position: relative; /* 移除动画时离场行绝对定位,其余行平滑补位 */
}
.q-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 10px;
  border-radius: 9px;
  cursor: pointer;
  transition: background 0.18s ease;
}
.q-row:hover {
  background: var(--np-btn-bg-hover);
}
.q-row.now {
  cursor: default;
  background: var(--np-btn-bg);
}
.q-idx {
  width: 14px;
  flex: none;
  text-align: center;
  font-size: 11px;
  color: var(--np-fg-3);
  font-variant-numeric: tabular-nums;
}
/* 正在播放行:三根跳动的音量条,暂停时定格 */
.q-eq {
  width: 14px;
  height: 14px;
  flex: none;
  display: flex;
  align-items: flex-end;
  justify-content: center;
  gap: 2px;
}
.q-eq i {
  width: 3px;
  border-radius: 1px;
  background: var(--accent);
  animation: q-eq 0.9s ease-in-out infinite;
}
.q-eq i:nth-child(2) {
  animation-delay: 0.28s;
}
.q-eq i:nth-child(3) {
  animation-delay: 0.55s;
}
@keyframes q-eq {
  0%,
  100% {
    height: 4px;
  }
  50% {
    height: 13px;
  }
}
.q-row.now.paused .q-eq i {
  animation-play-state: paused;
}
.q-meta {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}
.q-t {
  font-size: 12.5px;
  font-weight: 600;
  color: var(--np-fg);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.q-row.now .q-t {
  color: var(--np-fg-strong);
}
.q-a {
  font-size: 10.5px;
  color: var(--np-fg-3);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.q-dur {
  flex: none;
  font-size: 10.5px;
  color: var(--np-fg-3);
  font-variant-numeric: tabular-nums;
}
.q-remove {
  flex: none;
  width: 24px;
  height: 24px;
  border-radius: 6px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--np-fg-3);
  opacity: 0;
  transition: opacity 0.15s ease, background 0.15s ease, color 0.15s ease;
}
.q-remove svg {
  width: 11px;
  height: 11px;
}
.q-row:hover .q-remove {
  opacity: 1;
}
.q-remove:hover {
  background: var(--np-btn-bg-hover);
  color: var(--danger);
}
.q-sep {
  padding: 14px 10px 6px;
  font-size: 10.5px;
  font-weight: 600;
  letter-spacing: 0.08em;
  color: var(--np-fg-mute);
}
.q-empty {
  padding: 28px 10px;
  text-align: center;
  font-size: 12px;
  color: var(--np-fg-mute);
}
/* 抽屉滑入/列表增删的过渡 */
.drawer-enter-active,
.drawer-leave-active {
  transition: transform 0.38s var(--ease-out-soft), opacity 0.28s ease;
}
.drawer-enter-from,
.drawer-leave-to {
  transform: translateX(48px);
  opacity: 0;
}
.qlist-move {
  transition: transform 0.3s var(--ease-out-soft);
}
.qlist-enter-active {
  transition: opacity 0.25s ease, transform 0.3s var(--ease-out-soft);
}
.qlist-leave-active {
  position: absolute;
  width: calc(100% - 20px);
  transition: opacity 0.2s ease, transform 0.3s var(--ease-out-soft);
}
.qlist-enter-from,
.qlist-leave-to {
  opacity: 0;
  transform: translateX(20px);
}

/* ===== 纸质皮肤:播放页变"印刷节目单"——纸底、墨字、衬线歌词、封面装裱 ===== */
html[data-skin="paper"] .np {
  --np-bg: var(--bg);
  --np-fg: var(--text);
  --np-fg-2: var(--text-2);
  --np-fg-3: var(--text-3);
  --np-fg-soft: var(--ink);
  --np-fg-strong: var(--ink);
  --np-fg-mute: var(--ink-2);
  --np-plain: var(--text);
  --np-btn-bg: transparent;
  --np-btn-bg-hover: var(--hover);
  --np-close-bg: transparent;
  --np-close-hover: var(--hover);
  --np-close-fg: var(--ink);
  --np-track: rgba(31, 30, 29, 0.18);
  --np-thumb: var(--ink);
  --np-art-shadow: var(--paper-shadow);
  --np-art-bg: var(--bg-3);
  --np-text-shadow: none;
  background: var(--bg);
}
html[data-skin="paper"] .np::before {
  display: none; /* 纸上不放模糊封面:节目单是干净的纸面 */
}
html[data-skin="paper"] .np .info {
  text-shadow: none;
}
html[data-skin="paper"] .np .info .title {
  font-family: var(--font-display);
}
html[data-skin="paper"] .np .lyrics {
  font-family: var(--font-display);
}
html[data-skin="paper"] .np .lyrics.synced p.active {
  color: var(--accent);
}
html[data-skin="paper"] .np .art {
  border: 1px solid var(--ink);
  box-shadow: var(--paper-shadow);
}
html[data-skin="paper"] .np .close {
  border: 1px solid var(--ink);
}
/* 队列抽屉:纸质下变印刷插页——纸面 + 墨线,去掉玻璃磨纱 */
html[data-skin="paper"] .np .queue {
  background: var(--bg-2);
  border-left: 1px solid var(--ink);
  backdrop-filter: none;
  -webkit-backdrop-filter: none;
}
html[data-skin="paper"] .np .q-row:hover {
  background: var(--hover);
}
html[data-skin="paper"] .np .q-row.now {
  background: var(--active);
}
html[data-skin="paper"] .np .q-close:hover,
html[data-skin="paper"] .np .q-remove:hover {
  background: var(--hover);
  color: var(--text);
}
html[data-skin="paper"] .np .q-remove:hover {
  color: var(--danger);
}
</style>