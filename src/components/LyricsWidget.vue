<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize, PhysicalPosition } from "@tauri-apps/api/window";

/** 桌面歌词小窗(独立 WebView 窗口,常驻后台):
 *  状态/歌词由主窗口经 media://state、media://lyrics 广播,操作经 media://action 回传;
 *  位置持久化在 localStorage(同源三窗口共享,storage 事件自动同步设置页)。
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

/* 进度可拖:拖动中本地预览,松手才提交——主窗口的 media 桥解析 "seek:<秒>";
   状态广播约 1Hz,不屏蔽会把滑块每秒拽回旧位置。提交后保留预览直到位置追平,
   避免松手瞬间闪回旧位置 */
const seeking = ref(false);
const seekPreview = ref<number | null>(null);
const shownPos = computed(() => seekPreview.value ?? state.value.position);
const pct = computed(() => {
  const d = state.value.duration;
  return d > 0 ? (Math.min(shownPos.value, d) / d) * 100 : 0;
});
function fmtTime(s: number): string {
  const v = Number.isFinite(s) && s > 0 ? Math.floor(s) : 0;
  return `${Math.floor(v / 60)}:${String(v % 60).padStart(2, "0")}`;
}
const seekingTime = computed(() => `${fmtTime(shownPos.value)} / ${fmtTime(state.value.duration)}`);
const showSeekTime = computed(() => seeking.value || seekPreview.value !== null);
let seekSettleTimer = 0;
function onSeekInput(e: Event): void {
  seeking.value = true;
  seekPreview.value = Number((e.target as HTMLInputElement).value);
  window.clearTimeout(seekSettleTimer);
}
function onSeekCommit(e: Event): void {
  const v = Number((e.target as HTMLInputElement).value);
  seeking.value = false;
  seekPreview.value = v;
  if (state.value.duration > 0) void emit("media://action", `seek:${v}`);
  // 广播回来前的兜底:1.5s 内仍未追平也要交还真实进度,防止预览永久滞留
  window.clearTimeout(seekSettleTimer);
  seekSettleTimer = window.setTimeout(() => {
    seekPreview.value = null;
  }, 1500);
}

/* 字号预算的固定开销(像素):上 padding 10 + 底部控制带 74
   + 三行时的行间 gap 12 + 进度条命中区 14。底部控制带 = 胶囊 54 + 底距 12
   + 与歌词间隔 8,由 .lyr-root 的 padding-bottom 预留,胶囊悬停浮现时不再压住歌词 */
const FIXED_OVERHEAD = 110;

/* 歌词大小档位:档位是**字号上限**,窗高按最坏三行(大字 + 译文 + 下一句)刚好容纳,
   两行歌词不会再被撑得比档位更大——旧版统一的 72px 上限让中/大两档在多数曲目下
   顶到同一字号,档位形同虚设。手动把窗拉得比档位更高时上限按比例放宽,
   保留「拉高窗口 = 放大歌词」的手感 */
const LYRIC_TIERS = {
  s: { h: 188, fs: 32 },
  m: { h: 208, fs: 40 },
  // 48 是默认 720 宽(内容区 676)能整行放下 14 个汉字的极限,再大就只剩省略号
  l: { h: 228, fs: 48 },
} as const;
type LyricTier = keyof typeof LYRIC_TIERS;
function savedTier(): LyricTier {
  const v = localStorage.getItem("tm-lyrics-scale");
  return v === "s" || v === "l" ? v : "m";
}
const lyricTier = ref<LyricTier>(savedTier());
const winH = ref(window.innerHeight);
/* 字号自适应:窗口高度摊给可见行(大字/译文/下一句)。
   固定开销是**像素**不是行数,按行数折算会在小窗时把字算大(总高超窗 → 上下切行);
   译文/下一句的系数 = 字号比(0.52/0.4) × 行高比(1.3/1.22) */
const fontSize = computed(() => {
  const rows = 1 + (mainTrans.value ? 0.554 : 0) + (nextLine.value ? 0.426 : 0);
  const tier = LYRIC_TIERS[lyricTier.value];
  const fit = (Math.max(winH.value, 100) - FIXED_OVERHEAD) / (rows * 1.22);
  // 档位给上限;手动拉高时上限同比放宽,保留放大能力
  const cap = tier.fs * Math.max(1, winH.value / tier.h);
  return Math.max(16, Math.min(fit, cap, 72));
});
function onResize(): void {
  winH.value = window.innerHeight;
}

/* 歌词颜色:默认跟随皮肤(正文取主题文字色、暖端取品牌 accent 令牌)——
   浅色主题下整句变墨色系,白壁纸才读得清;用户在设置页调配自定色后,
   自定色成为绝对主色(渐变两端都由它派生,译文/下一句/进度线同步跟随),
   不再从白色起笔——用户选了红就要看到红。storage 事件实时跟随设置页 */
const customColor = ref("");
function applyColorFromStorage(): void {
  const v = localStorage.getItem("tm-lyrics-color") ?? "";
  customColor.value = /^#[0-9a-fA-F]{6}$/.test(v) ? v : "";
}
const mainStyle = computed(() => {
  const c = customColor.value;
  if (!state.value.playing) {
    // 闲置:去渐变降亮示意停机;自定色时沿用同色系
    return c ? { WebkitTextFillColor: `color-mix(in srgb, ${c} 42%, #9a9a9a)` } : {};
  }
  return {
    // 必须用 backgroundImage 而非 background 简写:简写会重置 background-clip
    // (内联优先级又压过类里的裁切声明),clip 一旦失效文字填充透明、
    // 渐变铺满整框——「一播放歌词就消失」的根因。裁切一并内联钉死
    backgroundImage: c
      ? `linear-gradient(100deg, color-mix(in srgb, ${c} 38%, #fff) 8%, ${c} 92%)`
      : `linear-gradient(100deg, var(--text) 22%, color-mix(in srgb, var(--accent) 42%, var(--text)) 58%, color-mix(in srgb, var(--accent) 82%, var(--text)) 130%)`,
    WebkitBackgroundClip: "text",
    backgroundClip: "text",
  };
});
const transColor = computed(() => {
  const c = customColor.value;
  return c ? `color-mix(in srgb, ${c} 86%, #fff)` : `color-mix(in srgb, var(--text) 90%, transparent)`;
});
const nextColor = computed(() => {
  const c = customColor.value;
  return c ? `color-mix(in srgb, ${c} 62%, #fff)` : `color-mix(in srgb, var(--text) 66%, transparent)`;
});
const progressFill = computed(() => {
  const c = customColor.value;
  return `linear-gradient(90deg, color-mix(in srgb, var(--text) 80%, transparent), ${c || "var(--accent)"})`;
});
const knobStyle = computed(() => ({
  left: `${pct.value}%`,
  background: customColor.value || "var(--accent)",
}));
function onColorStorage(e: StorageEvent): void {
  if (e.key === "tm-lyrics-color") applyColorFromStorage();
  if (e.key === "tm-lyrics-scale") void applyScale();
}

/* 档位落地:切换档位只改窗高(宽度与左上角保持不动),字号上限由 lyricTier 决定 */
async function applyScale(): Promise<void> {
  lyricTier.value = savedTier();
  const h = LYRIC_TIERS[lyricTier.value].h;
  // 先钉字号基准再改窗:隐藏窗口的 setSize 不派发 resize 事件,
  // 不主动同步 winH 的话字号仍按旧窗高计算,新窗里直接溢出切行
  winH.value = h;
  try {
    const sf = await win.scaleFactor();
    const cur = await win.innerSize(); // 物理像素,换算回逻辑宽度保持不动
    await win.setSize(new LogicalSize(cur.width / sf, h));
  } catch {
    /* 尺寸调整失败不致命:字号自适应仍按钉住的窗高工作 */
  }
}

/* 旧版残留偏好键(底板浓度/字号档):对应控件已移除,启动即清掉,
   防设置页/调试困惑 */
localStorage.removeItem("tm-lyrics-opacity");
localStorage.removeItem("tm-lyrics-size");

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
      // 真实进度追平预览值后交还状态源;拖动中不打断用户
      if (
        !seeking.value
        && seekPreview.value !== null
        && Math.abs(e.payload.position - seekPreview.value) < 1.5
      ) {
        window.clearTimeout(seekSettleTimer);
        seekPreview.value = null;
      }
    }),
    await listen<WidgetLyrics>("media://lyrics", (e) => {
      lyrics.value = e.payload;
      // 切歌清掉上一首的进度预览
      window.clearTimeout(seekSettleTimer);
      seekPreview.value = null;
    }),
    await win.onMoved((e) => onMovedSave(e.payload.x, e.payload.y)),
  );
  void emit("media://state-request");
  applyColorFromStorage();
  void applyScale();
  window.addEventListener("keydown", onKey);
  window.addEventListener("storage", onColorStorage);
  window.addEventListener("resize", onResize);
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
  window.clearTimeout(seekSettleTimer);
  window.removeEventListener("keydown", onKey);
  window.removeEventListener("storage", onColorStorage);
  window.removeEventListener("resize", onResize);
});
</script>

<template>
  <!-- 整窗可拖(deep 规则下按钮自动豁免);窗口全透明,字号按可见行数自适应 -->
  <div class="lyr-root" data-tauri-drag-region="deep" :style="{ '--fs': fontSize + 'px' }">
    <div class="lyr-lines" data-tauri-drag-region="deep">
      <!-- 渐变与字色由 mainStyle/transColor 提供(皮肤令牌或用户自定色);
           闲置态不绑背景内联,让 .idle 的去渐变降白生效 -->
      <p
        class="line-main"
        :class="{ idle: !state.playing }"
        :style="mainStyle"
      >
        {{ mainLine }}
      </p>
      <p v-if="mainTrans" class="line-trans" :style="{ color: transColor }">{{ mainTrans }}</p>
      <p v-if="nextLine" class="line-next" :style="{ color: nextColor }">{{ nextLine }}</p>
      <!-- 进度线即 seek 控件:14px 透明命中区包住细线,悬停/拖动加粗并浮出圆点;
           用原生 range 当命中层,INPUT 属于可交互元素,不会被 deep 拖窗区吃掉 -->
      <div class="progress" :class="{ 'is-seeking': showSeekTime }">
        <div class="track">
          <div class="progress-fill" :style="{ width: pct + '%', background: progressFill }"></div>
        </div>
        <span v-if="state.duration > 0" class="knob" aria-hidden="true" :style="knobStyle"></span>
        <input
          class="seek"
          type="range"
          aria-label="播放进度"
          min="0"
          :max="state.duration || 1"
          step="0.1"
          title="点击或拖动调整播放进度"
          :value="shownPos"
          :disabled="!state.duration"
          @input="onSeekInput"
          @change="onSeekCommit"
        />
      </div>
    </div>

    <!-- 控制胶囊:悬停浮现,只留传输与关闭——音量与字号都跟随主窗口/设置页 -->
    <div class="lyr-ctl">
      <span class="song" :title="state.title">
        {{ showSeekTime ? seekingTime : state.title || "未在播放" }}
      </span>
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
   背景板在深色壁纸上必然糊成死黑(旧版教训)。投影压到单层耳语级:
   drop-shadow 叠层会互相再投影、浓度翻倍,大半径暗影本身就是一团
   「模糊背景」(用户反馈),这里只留最低限度给亮壁纸兜底,无可见暗晕 */
.lyr-root {
  position: relative; /* 操控胶囊的绝对定位锚点 */
  height: 100%;
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 8px;
  /* 底部 74 = 胶囊 54 + 底距 12 + 间隔 8:控制带是专属空带,歌词在其上方居中 */
  padding: 10px 22px 74px;
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
/* 当前句:大字重 + 品牌色渐变填充。字号按可见行数自适应(--fs,组件注入):
   行多字小、行少字大,任何窗高都完整可见,不再被 overflow:hidden 切行。
   填充透明化后 text-shadow 会透进字形,可读性用单层柔影兜底 */
.line-main {
  /* fit-content:渐变必须贴合文字宽度而非元素框——全宽元素上短句只路过
     渐变中段(大量混白),选红只见粉(用户反馈「这是啥颜色」) */
  width: fit-content;
  font-size: var(--fs);
  font-weight: 800;
  line-height: 1.22;
  text-align: center;
  white-space: nowrap;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  -webkit-background-clip: text;
  background-clip: text;
  -webkit-text-fill-color: transparent;
  filter: drop-shadow(0 2px 8px rgba(0, 0, 0, 0.18));
  transition: opacity 0.3s ease, filter 0.3s ease;
}
/* 对照译文:介于大字与预览行之间,不用渐变,不与大字抢主次;
   颜色经 transColor 内联(主题文字色或自定色派生),投影兜底对比 */
.line-trans {
  font-size: calc(var(--fs) * 0.52);
  font-weight: 650;
  line-height: 1.3;
  text-align: center;
  white-space: nowrap;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  text-shadow: 0 2px 8px rgba(0, 0, 0, 0.18);
}
.line-main.idle {
  /* 暂停/未播放:去渐变降亮示意停机;用主题文字色,浅色主题下为墨色 */
  background: none;
  -webkit-text-fill-color: color-mix(in srgb, var(--text) 55%, transparent);
  filter: drop-shadow(0 2px 8px rgba(0, 0, 0, 0.16));
}
/* 下一句:小一号的次级色,预告走向;颜色经 nextColor 内联 */
.line-next {
  font-size: calc(var(--fs) * 0.4);
  font-weight: 650;
  line-height: 1.3;
  text-align: center;
  white-space: nowrap;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  text-shadow: 0 2px 8px rgba(0, 0, 0, 0.15);
}

/* 进度条:视觉仍是贴在文字块下的细线,但整个 14px 高的区域都可点击/拖拽。
   细线本身只有 2.5px,直接当命中区几乎抓不住;加粗与圆点只在悬停/拖动时浮现 */
.progress {
  position: relative;
  width: min(60%, 320px);
  height: 14px;
  cursor: pointer;
}
.track {
  position: absolute;
  left: 0;
  right: 0;
  top: 50%;
  height: 2.5px;
  transform: translateY(-50%);
  border-radius: 999px;
  background: color-mix(in srgb, var(--text) 20%, transparent);
  overflow: hidden;
  transition: height 0.16s ease;
}
.progress:hover .track,
.progress.is-seeking .track,
.progress:focus-within .track {
  height: 5px;
}
.progress-fill {
  height: 100%;
  border-radius: inherit;
  background: linear-gradient(90deg, color-mix(in srgb, var(--text) 80%, transparent), var(--accent));
  transition: width 0.25s linear;
}
/* 拖动中进度要跟手,不能吃 0.25s 的缓动 */
.progress.is-seeking .progress-fill {
  transition: none;
}
.knob {
  position: absolute;
  top: 50%;
  width: 10px;
  height: 10px;
  border-radius: 50%;
  transform: translate(-50%, -50%);
  opacity: 0;
  pointer-events: none;
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.35);
  transition: opacity 0.18s ease;
}
.progress:hover .knob,
.progress.is-seeking .knob,
.progress:focus-within .knob {
  opacity: 1;
}
/* 透明原生 range 只负责命中与键盘操作,视觉完全由 track/knob 承担 */
.seek {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  margin: 0;
  padding: 0;
  border: 0;
  opacity: 0;
  cursor: pointer;
  -webkit-appearance: none;
  appearance: none;
  background: transparent;
}
.seek:disabled {
  cursor: default;
}

/* ===== 控制胶囊:绝对定位悬停在底缘的专属控制带里——
   歌词区用 padding-bottom 让出 74px,胶囊默认隐身且不接指针
   (不挡拖拽),悬停窗口才浮现可点 ===== */
.lyr-ctl {
  position: absolute;
  bottom: 12px;
  left: 50%;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 14px;
  border-radius: 999px;
  /* 胶囊配色走皮肤令牌:深色主题近黑烟熏、浅色/纸质为米白纸片,
     三套皮肤自动换装;半透明保留悬浮感,外投影维持与桌面的分层 */
  background: color-mix(in srgb, var(--bg-2) 88%, transparent);
  border: 1px solid var(--hairline);
  box-shadow:
    inset 0 1px 0 color-mix(in srgb, var(--text) 10%, transparent),
    0 8px 28px rgba(0, 0, 0, 0.4);
  opacity: 0;
  pointer-events: none;
  transform: translate(-50%, 6px);
  transition: opacity 0.28s var(--ease-out-soft), transform 0.28s var(--ease-out-soft);
}
.lyr-root:hover .lyr-ctl {
  opacity: 1;
  pointer-events: auto;
  transform: translate(-50%, 0);
}
/* 胶囊行首:平时是歌曲名(超长省略),拖动进度时临时显示时间。
   固定宽度:歌名与时间切换时胶囊不跳宽,传输键不会被挤着左右移动 */
.song {
  width: 110px;
  font-size: 12px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.vline {
  width: 1px;
  height: 16px;
  flex: none;
  background: var(--hairline);
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
  color: var(--text-2);
  transition: color 0.18s ease, background 0.18s ease, transform 0.25s var(--ease-spring);
}
.cbtn svg {
  width: 15px;
  height: 15px;
}
.cbtn:hover {
  color: var(--text);
  background: var(--hover);
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
</style>
