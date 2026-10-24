<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { ui, setThemeMode, setSkin } from "../stores/ui";
import { lib, flashStatus, openImportDir, addFolder, rescan, importTmcPick, importMusicPick } from "../stores/library";
import { statsEnabled, setStatsEnabled, clearStats } from "../stores/stats";
import { lyricsOn, toggleLyricsWindow } from "../stores/media";
import { api, type AssocState } from "../api";

const skinOptions = [
  { value: "glass", label: "液态玻璃" },
  { value: "paper", label: "纸质" },
] as const;

/** 桌面歌词小窗开关:状态与切换逻辑在 media store,播放页控制行共用同一份 */
async function toggleLyrics(): Promise<void> {
  const err = await toggleLyricsWindow();
  if (err) flashStatus(`歌词小窗切换失败: ${err}`);
}

/** 桌面歌词自定色:空 = 跟随皮肤(渐变暖端取品牌 accent)。
 *  写入 localStorage 即经 storage 事件同步到歌词小窗,实时生效 */
const lyricsColor = ref(localStorage.getItem("tm-lyrics-color") ?? "");
function onLyricsColor(e: Event): void {
  const v = (e.target as HTMLInputElement).value;
  lyricsColor.value = v;
  localStorage.setItem("tm-lyrics-color", v);
}
function resetLyricsColor(): void {
  lyricsColor.value = "";
  localStorage.removeItem("tm-lyrics-color");
}

const themeOptions = [
  { value: "system", label: "跟随系统" },
  { value: "light", label: "浅色" },
  { value: "dark", label: "黑色" },
] as const;

const FORMAT_HINT = "支持 MP3 / FLAC / M4A / OGG / OPUS / WAV · 歌词支持内嵌或同名 .lrc · 封面自动识别";
const TMC_HINT =
  "TMC 音乐包为标准 7z 压缩包:内含音频、同名 .lrc 歌词、cover.jpg/png 封面与 meta.json 元数据。把 .tmc 拖进窗口即可导入;在歌曲列表悬停某行可一键导出 TMC。TMCL 播放列表(.tmcl)同为 7z,内含整单音乐源文件与 playlist.json,在播放列表页导出,拖进窗口即导入。";
const ASSOC_HINT =
  "勾选后 TauriMusic 出现在右键\"打开方式\"列表,并注册为当前用户的双击默认程序。若双击仍由其他应用打开,说明系统记住了旧默认:请在\"设置 → 默认应用\"中调整,或在文件右键\"打开方式 → 选择其他应用\"勾选\"始终\"。";

const assocs = ref<AssocState[]>([]);
const autostart = ref(false);

onMounted(async () => {
  try {
    assocs.value = await api.getAssociations();
  } catch (err) {
    console.error("读取格式关联失败", err);
  }
  try {
    autostart.value = await api.getAutostart();
  } catch (err) {
    console.error("读取自启动状态失败", err);
  }
});

async function toggleAutostart(): Promise<void> {
  const target = !autostart.value;
  try {
    await api.setAutostart(target);
    autostart.value = target;
  } catch (err) {
    console.error("修改自启动失败", err);
    flashStatus(`自启动修改失败: ${String(err)}`);
  }
}

async function toggleAssoc(a: AssocState): Promise<void> {
  try {
    await api.setAssociation(a.ext, !a.registered);
    a.registered = !a.registered;
  } catch (err) {
    console.error("修改格式关联失败", err);
    flashStatus(`格式关联修改失败: ${String(err)}`);
  }
}

// 清除统计数据:两段确认(3 秒内再点一次才执行),防止误触毁掉全部统计
const clearArmed = ref(false);
let clearTimer = 0;
onUnmounted(() => window.clearTimeout(clearTimer));

async function clearStatsConfirmed(): Promise<void> {
  if (!clearArmed.value) {
    clearArmed.value = true;
    window.clearTimeout(clearTimer);
    clearTimer = window.setTimeout(() => (clearArmed.value = false), 3000);
    return;
  }
  clearArmed.value = false;
  window.clearTimeout(clearTimer);
  try {
    await clearStats();
    flashStatus("统计数据已清除");
  } catch (err) {
    console.error("清除统计失败", err);
    flashStatus(`清除失败: ${String(err)}`);
  }
}
</script>

<template>
  <div class="view">
    <header class="view-head">
      <h1>设置</h1>
    </header>

    <div class="col">
      <!-- 通用 -->
      <section class="sect">
        <div class="sect-head">
          <span class="tint" style="--tint: #98989d">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round">
              <path d="M5 7.5h7M16.5 7.5H19M5 12h3M11.5 12H19M5 16.5h7M16.5 16.5H19" />
              <circle cx="14" cy="7.5" r="2" /><circle cx="9.5" cy="12" r="2" /><circle cx="14" cy="16.5" r="2" />
            </svg>
          </span>
          <span>通用</span>
        </div>
        <div class="group">
          <button class="opt" @click="toggleAutostart">
            <span class="label">开机自启动<small class="sub">登录 Windows 后自动启动,托盘常驻</small></span>
            <span class="switch" :class="{ on: autostart }"><i></i></span>
          </button>
        </div>
      </section>

      <!-- 统计与隐私 -->
      <section class="sect">
        <div class="sect-head">
          <span class="tint" style="--tint: #0a84ff">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round">
              <path d="M5 20v-8M12 20V4M19 20v-5" />
            </svg>
          </span>
          <span>统计与隐私</span>
        </div>
        <div class="group">
          <button class="opt" @click="setStatsEnabled(!statsEnabled)">
            <span class="label">听歌统计<small class="sub">记录本地与在线收听时长和次数,统计页可分开查看;数据仅保存在本机,绝不联网上传。关闭后停止统计并隐藏统计页</small></span>
            <span class="switch" :class="{ on: statsEnabled }"><i></i></span>
          </button>
          <button class="opt" :class="{ armed: clearArmed }" @click="clearStatsConfirmed">
            <span class="label">清除统计数据<small class="sub">{{ clearArmed ? "再点一次确认清除,此操作不可恢复" : "删除本机全部听歌统计,不可恢复" }}</small></span>
            <span class="act" :class="{ armed: clearArmed }">{{ clearArmed ? "确认清除" : "清除" }}</span>
          </button>
        </div>
      </section>

      <!-- 皮肤 -->
      <section class="sect">
        <div class="sect-head">
          <span class="tint" style="--tint: #ff9f0a">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
              <path d="M12 3s6 6.3 6 10.1a6 6 0 0 1-12 0C6 9.3 12 3 12 3z" />
            </svg>
          </span>
          <span>皮肤</span>
        </div>
        <div class="group">
          <div class="opt static">
            <span class="label">界面材质<small class="sub">液态玻璃:封面氛围驱动的透光材质 · 纸质:印刷节目单质感</small></span>
            <div class="seg">
              <button v-for="s in skinOptions" :key="s.value" :class="{ on: ui.skin === s.value }" @click="setSkin(s.value)">
                {{ s.label }}
              </button>
            </div>
          </div>
        </div>
      </section>

      <!-- 外观 -->
      <section class="sect">
        <div class="sect-head">
          <span class="tint" style="--tint: #5e5ce6">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
              <path d="M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z" />
            </svg>
          </span>
          <span>外观</span>
        </div>
        <div class="group">
          <div class="opt static">
            <span class="label">主题<small class="sub">跟随系统明暗自动切换</small></span>
            <div class="seg">
              <button v-for="opt in themeOptions" :key="opt.value" :class="{ on: ui.themeMode === opt.value }" @click="setThemeMode(opt.value)">
                {{ opt.label }}
              </button>
            </div>
          </div>
          <div class="opt">
            <button class="lyr-toggle" @click="toggleLyrics">
              <span class="label">桌面歌词<small class="sub">桌面置顶小窗:歌词大字直接浮于桌面,颜色可在右侧调配,关闭主窗口仍在</small></span>
              <span class="switch" :class="{ on: lyricsOn }"><i></i></span>
            </button>
            <span class="lyr-color" title="歌词颜色">
              <input type="color" :value="lyricsColor || '#f0a08e'" @input="onLyricsColor" />
              <button v-if="lyricsColor" class="lyr-color-reset" title="恢复跟随皮肤" @click="resetLyricsColor">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round">
                  <path d="M6 6l12 12M18 6L6 18" />
                </svg>
              </button>
            </span>
          </div>
        </div>
      </section>

      <!-- 导入 -->
      <section class="sect">
        <div class="sect-head">
          <span class="tint" style="--tint: #30d158">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
              <path d="M12 3v10m0 0 4-4m-4 4-4-4" />
              <path d="M4 17v2.2c0 1 .8 1.8 1.8 1.8h12.4c1 0 1.8-.8 1.8-1.8V17" />
            </svg>
          </span>
          <span>导入</span>
        </div>
        <div class="group">
          <div class="opt static col-wrap">
            <span class="label">导入文件夹<small class="sub">{{ FORMAT_HINT }}</small></span>
            <div class="btns">
              <button class="pill pill--hover mini" @click="addFolder">添加音乐文件夹</button>
              <button class="pill pill--hover mini" @click="openImportDir">打开</button>
            </div>
            <p class="dir" :title="lib.importDir">{{ lib.importDir }}</p>
          </div>
          <div class="opt static">
            <span class="label">导入音乐文件<small class="sub">支持多选;导入后自动通过网易云公开接口匹配封面与歌词。APE / WMA 等非常规格式自动经内置 ffmpeg 转为无损 FLAC,原文件保持不动</small></span>
            <div class="btns">
              <button class="pill pill--hover mini" @click="importMusicPick">选择文件</button>
            </div>
          </div>
          <div class="opt static col-wrap">
            <span class="label">TMC 音乐包<small class="sub">{{ TMC_HINT }}</small></span>
            <div class="btns">
              <button class="pill pill--hover mini" @click="importTmcPick">导入 TMC / TMCL</button>
            </div>
          </div>
        </div>
      </section>

      <!-- 资料库 -->
      <section class="sect">
        <div class="sect-head">
          <span class="tint" style="--tint: #ff375f">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
              <path d="M20 6.5 9 9v8.2" /><circle cx="6.6" cy="17.6" r="2.5" /><circle cx="17.5" cy="14.8" r="2.5" />
            </svg>
          </span>
          <span>资料库</span>
        </div>
        <div class="group">
          <div class="opt static">
            <span class="label">曲库规模<small class="sub">{{ lib.tracks.length }} 首歌曲 · {{ lib.folders.length }} 个文件夹 · 重新扫描以发现外部目录的变更</small></span>
            <div class="btns">
              <button class="pill pill--hover mini" :disabled="lib.scanning" @click="rescan">
                {{ lib.scanning ? `扫描中 ${lib.scanCurrent}/${lib.scanTotal}` : "重新扫描" }}
              </button>
            </div>
          </div>
        </div>
      </section>

      <!-- 格式关联 -->
      <section class="sect">
        <div class="sect-head">
          <span class="tint" style="--tint: #64d2ff">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
              <path d="m9.5 14.5 5-5" />
              <path d="M8 12l-2.1 2.1a3.4 3.4 0 0 0 4.8 4.8L13 16.7M16 12l2.1-2.1a3.4 3.4 0 0 0-4.8-4.8L11 8" />
            </svg>
          </span>
          <span>格式关联</span>
        </div>
        <div class="group">
          <button v-for="a in assocs" :key="a.ext" class="opt" @click="toggleAssoc(a)">
            <span class="label">{{ a.label }}<small class="sub">双击 .{{ a.ext }} 文件用 TauriMusic 打开</small></span>
            <span class="switch" :class="{ on: a.registered }"><i></i></span>
          </button>
          <p class="hint">{{ ASSOC_HINT }}</p>
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
.view {
  height: 100%;
  overflow-y: auto;
  padding: 22px 28px 48px;
}
/* Apple 设置式居中窄列 */
.col {
  max-width: 660px;
  margin: 10px auto 0;
}

/* 节标题:彩色应用图标 + 灰字,系统设置的分类语言 */
.sect {
  margin-bottom: 22px;
}
.sect-head {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 0 2px 8px;
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
}
.tint {
  width: 27px;
  height: 27px;
  flex: none;
  border-radius: 7px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--tint);
  color: #fff;
}
.tint svg {
  width: 15px;
  height: 15px;
}

/* 分组卡片:实底面 + 行间发丝线(iOS 列表组) */
.group {
  border: 1px solid var(--hairline);
  border-radius: 12px;
  background: var(--bg-2);
  overflow: hidden;
}
.group > * + * {
  border-top: 1px solid var(--hairline);
}

/* 行:标签左,控件右 */
.opt {
  display: flex;
  align-items: center;
  gap: 12px;
  width: 100%;
  padding: 10px 14px;
  text-align: left;
  transition: background 0.18s var(--ease-out-soft);
}
button.opt:hover {
  background: var(--hover);
}
.opt.static {
  cursor: default;
}
.opt.col-wrap {
  flex-wrap: wrap;
  row-gap: 8px;
}
/* 桌面歌词行:开关钮 + 取色器并排。行本体是 div,悬停反馈由内层开关钮承担 */
.lyr-toggle {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 12px;
  text-align: left;
  padding: 4px 8px;
  margin: -4px -8px;
  border-radius: 8px;
  transition: background 0.18s var(--ease-out-soft);
}
.lyr-toggle:hover {
  background: var(--hover);
}
.lyr-color {
  flex: none;
  display: flex;
  align-items: center;
  gap: 4px;
}
.lyr-color input[type="color"] {
  width: 28px;
  height: 28px;
  padding: 0;
  border: 1px solid var(--hairline);
  border-radius: 8px;
  background: none;
  cursor: pointer;
}
.lyr-color input[type="color"]::-webkit-color-swatch-wrapper {
  padding: 2px;
}
.lyr-color input[type="color"]::-webkit-color-swatch {
  border: none;
  border-radius: 5px;
}
.lyr-color-reset {
  width: 22px;
  height: 22px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-3);
}
.lyr-color-reset:hover {
  color: var(--text);
  background: var(--hover);
}
.lyr-color-reset svg {
  width: 11px;
  height: 11px;
}
.label {
  flex: 1;
  min-width: 0;
  font-size: 13px;
  color: var(--text);
  display: block;
}
.sub {
  display: block;
  font-size: 11px;
  color: var(--text-3);
  margin-top: 2px;
  line-height: 1.55;
}

/* iOS 式开关:轨道滑块,开启时 accent 着色 */
.switch {
  flex: none;
  width: 42px;
  height: 25px;
  border-radius: 999px;
  background: var(--pill-bg);
  position: relative;
  transition: background 0.22s var(--ease-out-soft);
}
.switch i {
  position: absolute;
  top: 2.5px;
  left: 2.5px;
  width: 20px;
  height: 20px;
  border-radius: 50%;
  background: #fff;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
  transition: transform 0.22s var(--ease-out-soft);
}
.switch.on {
  background: var(--accent);
}
.switch.on i {
  transform: translateX(17px);
}
button.opt:active .switch i {
  width: 23px;
}
button.opt:active .switch.on i {
  transform: translateX(14px);
}

/* 分段控制器:轨道与段的基础形态走全局 .seg,这里只留方圆角与浮起白段 */
.seg {
  border-radius: 9px;
}
.seg button {
  padding: 4.5px 13px;
  border-radius: 7px;
}
.seg button.on {
  background: var(--bg-2);
  color: var(--text);
  font-weight: 600;
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.18);
}

/* 清除统计的行动字:两段确认时转警示色 */
.act {
  flex: none;
  font-size: 12px;
  font-weight: 600;
  color: var(--accent);
}
.act.armed,
.opt.armed .label,
.opt.armed .sub {
  color: var(--danger);
}

/* 导入行的小按钮组与路径 */
.btns {
  display: flex;
  gap: 6px;
  flex: none;
}
/* 迷你胶囊:基类 .pill 之上只留尺寸(悬停换底/禁用淡化走基类) */
.mini {
  flex: none;
  padding: 5px 14px;
  font-size: 12px;
}
.dir {
  width: 100%;
  font-size: 11.5px;
  color: var(--text-3);
  padding: 0 2px;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  direction: rtl;
  text-align: left;
}
.sub.ext {
  overflow: hidden;
  text-overflow: ellipsis;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
}
</style>
