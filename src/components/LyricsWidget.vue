<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWindow, PhysicalPosition } from "@tauri-apps/api/window";

/** 桌面歌词小窗(独立 WebView 窗口,常驻后台):
 *  状态/歌词由主窗口经 media://state、media://lyrics 广播,操作经 media://action 回传;
 *  字号与位置持久化在 localStorage(同源三窗口共享,storage 事件自动同步设置页)。
 *  设计语言:「沉浸文字」——窗口全透明,歌词大字直接浮在桌面上,不设底板:
 *  任何形式的背景板在深色壁纸上都会糊成一整块死黑(旧版教训),
 *  可读性完全交给多层投影;控制条是悬停才浮现的烟熏玻璃胶囊 */

interface WidgetState {
  title: string;
  artist: string;
  playing: boolean;
  position: number;
  duration: number;
  volume: number;
}

interface WidgetLyrics {
  title: string;
  artist: string;
  synced: { timeMs: number; text: string; translation: string | null }[];
  plain: string | null;
}

const win = getCurrentWindow();

const state = ref<WidgetState>({
  title: "",
  artist: "",
  playing: false,
  position: 0,
  duration: 0,
  volume: 1,
});
const lyrics = ref<WidgetLyrics>({ title: "", artist: "", synced: [], plain: null });

/* 歌词高亮:与主窗口播放页同款二分查找 */
const activeIdx = computed(() => {
  const lines = lyrics.value.synced;
  if (!lines.length) return -1;
  const t = state.value.position * 1000 + 200;
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
  return idx;
});
const mainLine = computed(() => {
  if (activeIdx.value >= 0) return lyrics.value.synced[activeIdx.value].text || "♪";
  return state.value.title || "TauriMusic";
});
/* 当前句对照译文:有译文的双语歌词在大字与预览行之间多一层 */
const mainTrans = computed(() => {
  if (activeIdx.value >= 0) return lyrics.value.synced[activeIdx.value].translation;
  return null;
});
const nextLine = computed(() => {
  if (activeIdx.value >= 0 && activeIdx.value + 1 < lyrics.value.synced.length)
    return lyrics.value.synced[activeIdx.value + 1].text;
  return state.value.artist || "未在播放";
});

/* 细进度线:position/duration,缓动跟随 */
const pct = computed(() =>
  state.value.duration > 0 ? (state.value.position / state.value.duration) * 100 : 0,
);

/* 音量:拖动中本地预览,经 volume:<0~1> 动作回传主窗口实时生效 */
const volDragging = ref(false);
const volLocal = ref(1);
const volShow = computed(() => (volDragging.value ? volLocal.value : state.value.volume));
function onVolInput(e: Event): void {
  const v = Number((e.target as HTMLInputElement).value);
  volDragging.value = true;
  volLocal.value = v;
  void emit("media://action", `volume:${v}`);
}
function onVolCommit(e: Event): void {
  const v = Number((e.target as HTMLInputElement).value);
  volDragging.value = false;
  void emit("media://action", `volume:${v}`);
}

/* 旧版「烟熏底板浓度」键:底板已移除,启动即清掉残留,防设置页/调试困惑 */
localStorage.removeItem("tm-lyrics-opacity");

/* 歌词大小:0.6~1.6 缩放,作用于主/副行字号 */
const fsize = ref(clampSize(Number(localStorage.getItem("tm-lyrics-size") ?? "1")));
function clampSize(v: number): number {
  return Number.isFinite(v) ? Math.min(1.6, Math.max(0.6, v)) : 1;
}
function onSize(e: Event): void {
  fsize.value = clampSize(Number((e.target as HTMLInputElement).value));
}
function onSizeCommit(): void {
  localStorage.setItem("tm-lyrics-size", String(fsize.value));
}

function act(a: string): void {
  void emit("media://action", a);
}
function closeWidget(): void {
  // 与设置页的开关共享同一份 localStorage(storage 事件自动同步)
  localStorage.setItem("tm-lyrics-on", "0");
  void win.hide();
}
function onKey(e: KeyboardEvent): void {
  if (e.key === "Escape") closeWidget();
}

/* 位置记忆:拖动结束后落盘,下次显示自动回到原位 */
let moveTimer = 0;
function onMovedSave(x: number, y: number): void {
  window.clearTimeout(moveTimer);
  moveTimer = window.setTimeout(() => {
    localStorage.setItem("tm-lyrics-pos", JSON.stringify({ x, y }));
  }, 400);
}

let cleanups: (() => void)[] = [];

onMounted(async () => {
  cleanups.push(
    await listen<WidgetState>("media://state", (e) => {
      state.value = e.payload;
    }),
    await listen<WidgetLyrics>("media://lyrics", (e) => {
      lyrics.value = e.payload;
    }),
    await win.onMoved((e) => onMovedSave(e.payload.x, e.payload.y)),
  );
  void emit("media://state-request");
  window.addEventListener("keydown", onKey);
  // 恢复上次位置(物理像素,多显示器换算交给系统)
  try {
    const saved = localStorage.getItem("tm-lyrics-pos");
    if (saved) {
      const p = JSON.parse(saved) as { x: number; y: number };
      void win.setPosition(new PhysicalPosition(p.x, p.y));
    }
  } catch {
    /* 位置损坏时用默认位置 */
  }
});
onUnmounted(() => {
  cleanups.forEach((fn) => fn());
  cleanups = [];
  window.clearTimeout(moveTimer);
  window.removeEventListener("keydown", onKey);
});
</script>

<template>
  <!-- 整窗可拖(deep 规则下按钮/滑杆自动豁免);窗口全透明,歌词无底板直接浮于桌面 -->
  <div
    class="lyr-root"
    data-tauri-drag-region="deep"
    :style="{ '--fs': fsize }"
  >
    <div class="lyr-lines" data-tauri-drag-region="deep">
      <p class="line-main" :class="{ idle: !state.playing }">{{ mainLine }}</p>
      <p v-if="mainTrans" class="line-trans">{{ mainTrans }}</p>
      <p v-if="nextLine" class="line-next">{{ nextLine }}</p>
      <div class="progress" aria-hidden="true">
        <div class="progress-fill" :style="{ width: pct + '%' }"></div>
      </div>
    </div>

    <!-- 控制胶囊:悬停浮现,烟熏玻璃与托盘卡同语言;行首显示当前歌曲 -->
    <div class="lyr-ctl">
      <span class="song" :title="state.title">{{ state.title || "未在播放" }}</span>
      <span class="vline"></span>
      <button class="cbtn" title="上一曲" @click="act('prev')">
        <svg viewBox="0 0 24 24" fill="currentColor">
          <path d="M11.4 6.4v11.2L3.6 12z" />
          <path d="M20 6.4v11.2L12.2 12z" />
        </svg>
      </button>
      <button
        class="cbtn play"
        :title="state.playing ? '暂停' : '播放'"
        @click="act('toggle')"
      >
        <svg v-if="state.playing" viewBox="0 0 24 24" fill="currentColor">
          <rect x="7.2" y="5.5" width="3.4" height="13" rx="1.2" />
          <rect x="13.4" y="5.5" width="3.4" height="13" rx="1.2" />
        </svg>
        <svg v-else viewBox="0 0 24 24" fill="currentColor">
          <path d="M8.2 5.5v13L19 12z" />
        </svg>
      </button>
      <button class="cbtn" title="下一曲" @click="act('next')">
        <svg viewBox="0 0 24 24" fill="currentColor">
          <path d="M12.6 6.4v11.2L20.4 12z" />
          <path d="M4 6.4v11.2L11.8 12z" />
        </svg>
      </button>

      <span class="gap"></span>

      <svg class="cico" viewBox="0 0 24 24" fill="currentColor">
        <path d="M11.2 4.8 6.8 8.3H4v7.4h2.8l4.4 3.5z" />
        <path
          d="M14.5 9.3a3.9 3.9 0 0 1 0 5.4M17 7a7.2 7.2 0 0 1 0 10"
          fill="none"
          stroke="currentColor"
          stroke-width="1.7"
          stroke-linecap="round"
        />
      </svg>
      <input
        class="mini-slider"
        type="range"
        min="0"
        max="1"
        step="0.01"
        :value="volShow"
        :style="{ '--fill': volShow * 100 + '%' }"
        title="音量"
        @input="onVolInput"
        @change="onVolCommit"
      />

      <span class="gap"></span>

      <svg class="cico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round">
        <path d="M4 7h16M6 12h13M8 17h10" />
      </svg>
      <input
        class="mini-slider"
        type="range"
        min="0.6"
        max="1.6"
        step="0.05"
        :value="fsize"
        :style="{ '--fill': ((fsize - 0.6) / 1.0) * 100 + '%' }"
        title="歌词大小"
        @input="onSize"
        @change="onSizeCommit"
      />

      <span class="gap"></span>

      <button class="cbtn" title="关闭小窗(可在设置里重新开启)" @click="closeWidget">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round">
          <path d="M6 6l12 12M18 6L6 18" />
        </svg>
      </button>
    </div>
  </div>
</template>

<style scoped>
/* 沉浸文字模式:窗口全透明(配置 transparent),无任何底板——
   背景板在深色壁纸上必然糊成死黑(旧版教训),可读性由投影承担。
   投影刻意只用大半径低透明度的环境影:近距深影(哪怕 1px)贴着字形
   就是一圈黑描边,亮色壁纸上尤其明显(用户反馈「隐隐一圈黑」);
   柔和大影既能把字从亮背景上托出来,又不留下可见的暗边 */
.lyr-root {
  height: 100%;
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 8px;
  padding: 10px 22px 12px;
  user-select: none;
}

/* ===== 歌词区 ===== */
.lyr-lines {
  flex: none;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  overflow: hidden;
}
/* 当前句:大字重 + 品牌色渐变填充,字号随 --fs 缩放。
   填充透明化后 text-shadow 会透进字形,可读性改用 drop-shadow 滤镜:
   近距深影压住字形边缘,中距与远距环境影把字从亮壁纸上托出来 */
.line-main {
  font-size: calc(clamp(22px, 16vh, 38px) * var(--fs, 1));
  font-weight: 800;
  line-height: 1.22;
  text-align: center;
  white-space: nowrap;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  background: linear-gradient(100deg, #ffffff 34%, #ffe9e2 58%, #f0a08e 120%);
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
  filter: drop-shadow(0 2px 6px rgba(0, 0, 0, 0.32)) drop-shadow(0 10px 30px rgba(0, 0, 0, 0.2));
  transition: opacity 0.3s ease, filter 0.3s ease;
}
/* 对照译文:介于大字与预览行之间,近白但不用渐变,不与大字抢主次 */
.line-trans {
  font-size: calc(clamp(16px, 10vh, 24px) * var(--fs, 1));
  font-weight: 650;
  color: rgba(255, 255, 255, 0.9);
  text-align: center;
  white-space: nowrap;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  text-shadow:
    0 1px 6px rgba(0, 0, 0, 0.32),
    0 6px 22px rgba(0, 0, 0, 0.2);
}
.line-main.idle {
  /* 暂停/未播放:去渐变降白,示意停机 */
  background: none;
  -webkit-text-fill-color: rgba(255, 255, 255, 0.55);
  filter: drop-shadow(0 2px 6px rgba(0, 0, 0, 0.28));
}
/* 下一句:小一号的次级白,预告走向 */
.line-next {
  font-size: calc(clamp(13px, 8vh, 19px) * var(--fs, 1));
  font-weight: 650;
  color: rgba(255, 255, 255, 0.66);
  text-align: center;
  white-space: nowrap;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  text-shadow:
    0 1px 6px rgba(0, 0, 0, 0.3),
    0 5px 18px rgba(0, 0, 0, 0.18);
}

/* 细进度线:贴在文字块下的 2.5px 圆角线,随播放推进 */
.progress {
  width: min(60%, 320px);
  height: 2.5px;
  border-radius: 999px;
  background: rgba(255, 255, 255, 0.16);
  overflow: hidden;
  margin-top: 2px;
}
.progress-fill {
  height: 100%;
  border-radius: inherit;
  background: linear-gradient(90deg, rgba(255, 255, 255, 0.9), var(--accent));
  transition: width 0.25s linear;
}

/* ===== 控制胶囊:默认隐身,悬停窗口时从下方 6px 浮现 ===== */
.lyr-ctl {
  flex: none;
  margin: 0 auto;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 14px;
  border-radius: 999px;
  /* 烟熏玻璃胶囊:窗口背外无内容可采,用近实心渐变 + 顶缘高光模拟 */
  background: linear-gradient(180deg, rgba(50, 52, 56, 0.96) 0%, rgba(20, 21, 24, 0.96) 100%);
  border: 1px solid rgba(255, 255, 255, 0.14);
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.18),
    0 8px 28px rgba(0, 0, 0, 0.45);
  opacity: 0;
  transform: translateY(6px);
  transition: opacity 0.28s var(--ease-out-soft), transform 0.28s var(--ease-out-soft);
}
.lyr-root:hover .lyr-ctl {
  opacity: 1;
  transform: translateY(0);
}
/* 当前歌曲名:胶囊行首,超长省略 */
.song {
  font-size: 12px;
  font-weight: 600;
  color: rgba(255, 255, 255, 0.85);
  max-width: 110px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.vline {
  width: 1px;
  height: 16px;
  flex: none;
  background: rgba(255, 255, 255, 0.16);
}
.gap {
  width: 8px;
  flex: none;
}
.cbtn {
  width: 30px;
  height: 30px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: rgba(255, 255, 255, 0.72);
  transition: color 0.18s ease, background 0.18s ease, transform 0.25s var(--ease-spring);
}
.cbtn svg {
  width: 15px;
  height: 15px;
}
.cbtn:hover {
  color: #fff;
  background: rgba(255, 255, 255, 0.1);
}
.cbtn:active {
  transform: scale(0.88);
}
.cbtn.play {
  width: 38px;
  height: 38px;
  background: var(--accent);
  color: #fff;
  box-shadow: 0 3px 12px color-mix(in srgb, var(--accent) 45%, transparent);
}
.cbtn.play svg {
  width: 17px;
  height: 17px;
}
.cbtn.play:hover {
  background: var(--accent-hover);
  transform: scale(1.06);
}
.cico {
  width: 15px;
  height: 15px;
  flex: none;
  color: rgba(255, 255, 255, 0.55);
}
.mini-slider {
  width: 84px;
  height: 3px;
  border-radius: 999px;
  background: linear-gradient(
    to right,
    #fff var(--fill, 0%),
    rgba(255, 255, 255, 0.22) var(--fill, 0%)
  );
}
.mini-slider::-webkit-slider-thumb {
  -webkit-appearance: none;
  appearance: none;
  width: 9px;
  height: 9px;
  border-radius: 50%;
  background: #fff;
  box-shadow: 0 0 6px rgba(0, 0, 0, 0.4);
}
.mini-slider::-webkit-slider-thumb:hover {
  transform: scale(1.2);
}
</style>
