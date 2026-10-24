<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWindow, PhysicalPosition } from "@tauri-apps/api/window";

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

/* 细进度线:position/duration,缓动跟随 */
const pct = computed(() =>
  state.value.duration > 0 ? (state.value.position / state.value.duration) * 100 : 0,
);

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
    background: c
      ? `linear-gradient(100deg, color-mix(in srgb, ${c} 25%, #fff) 10%, ${c} 90%)`
      : `linear-gradient(100deg, var(--text) 22%, color-mix(in srgb, var(--accent) 42%, var(--text)) 58%, color-mix(in srgb, var(--accent) 82%, var(--text)) 130%)`,
  };
});
const transColor = computed(() => {
  const c = customColor.value;
  return c ? `color-mix(in srgb, ${c} 78%, #fff)` : `color-mix(in srgb, var(--text) 90%, transparent)`;
});
const nextColor = computed(() => {
  const c = customColor.value;
  return c ? `color-mix(in srgb, ${c} 52%, #fff)` : `color-mix(in srgb, var(--text) 66%, transparent)`;
});
const progressFill = computed(() => {
  const c = customColor.value;
  return `linear-gradient(90deg, color-mix(in srgb, var(--text) 80%, transparent), ${c || "var(--accent)"})`;
});
function onColorStorage(e: StorageEvent): void {
  if (e.key === "tm-lyrics-color") applyColorFromStorage();
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
    }),
    await listen<WidgetLyrics>("media://lyrics", (e) => {
      lyrics.value = e.payload;
    }),
    await win.onMoved((e) => onMovedSave(e.payload.x, e.payload.y)),
  );
  void emit("media://state-request");
  applyColorFromStorage();
  window.addEventListener("keydown", onKey);
  window.addEventListener("storage", onColorStorage);
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
  window.removeEventListener("storage", onColorStorage);
});
</script>

<template>
  <!-- 整窗可拖(deep 规则下按钮自动豁免);窗口全透明,歌词无底板直接浮于桌面 -->
  <div class="lyr-root" data-tauri-drag-region="deep">
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
      <div class="progress" aria-hidden="true">
        <div class="progress-fill" :style="{ width: pct + '%', background: progressFill }"></div>
      </div>
    </div>

    <!-- 控制胶囊:悬停浮现,只留传输与关闭——音量跟随主窗口,字号固定最大档 -->
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
/* 当前句:大字重 + 品牌色渐变填充,字号固定在原滑杆上限档。
   渐变由 mainFill 内联注入(默认取皮肤 accent 令牌,用户自定色覆盖暖端),
   闲置态不绑内联让 .idle 的去渐变降白生效。
   填充透明化后 text-shadow 会透进字形,可读性用单层柔影兜底 */
.line-main {
  font-size: clamp(34px, 25.6vh, 61px);
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
  font-size: clamp(25px, 16vh, 38px);
  font-weight: 650;
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
  font-size: clamp(20px, 12.8vh, 30px);
  font-weight: 650;
  text-align: center;
  white-space: nowrap;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  text-shadow: 0 2px 8px rgba(0, 0, 0, 0.15);
}

/* 细进度线:贴在文字块下的 2.5px 圆角线,随播放推进 */
.progress {
  width: min(60%, 320px);
  height: 2.5px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--text) 20%, transparent);
  overflow: hidden;
  margin-top: 2px;
}
.progress-fill {
  height: 100%;
  border-radius: inherit;
  background: linear-gradient(90deg, color-mix(in srgb, var(--text) 80%, transparent), var(--accent));
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
  /* 胶囊配色走皮肤令牌:深色主题近黑烟熏、浅色/纸质为米白纸片,
     三套皮肤自动换装;半透明保留悬浮感,外投影维持与桌面的分层 */
  background: color-mix(in srgb, var(--bg-2) 88%, transparent);
  border: 1px solid var(--hairline);
  box-shadow:
    inset 0 1px 0 color-mix(in srgb, var(--text) 10%, transparent),
    0 8px 28px rgba(0, 0, 0, 0.4);
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
  color: var(--text);
  max-width: 110px;
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
