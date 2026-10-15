<script setup lang="ts">
import { onMounted, ref } from "vue";
import { ui, setThemeMode, setSkin } from "../stores/ui";
import { lib, flashStatus, openImportDir, addFolder, rescan, importTmcPick, importMusicPick } from "../stores/library";
import { statsEnabled, setStatsEnabled, clearStats } from "../stores/stats";
import { api, type AssocState } from "../api";

const skinOptions = [
  { value: "glass", label: "液态玻璃" },
  { value: "paper", label: "纸质" },
] as const;

/** 桌面歌词小窗开关:开关状态持久化,窗口显示/隐藏由 Rust 命令执行;
 *  小窗自己的关闭按钮也会写同一份 localStorage(跨窗口 storage 事件同步) */
const lyricsOn = ref(localStorage.getItem("tm-lyrics-on") === "1");
function toggleLyrics(): void {
  const next = !lyricsOn.value;
  lyricsOn.value = next;
  localStorage.setItem("tm-lyrics-on", next ? "1" : "0");
  void (next ? api.showLyricsWindow() : api.hideLyricsWindow());
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
      <div class="sec">通用</div>
      <div class="group">
        <button class="opt" :class="{ on: autostart }" @click="toggleAutostart">
          <span>开机自启动<small class="opt-sub">登录 Windows 后自动启动,托盘常驻</small></span>
          <svg v-if="autostart" class="check" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
            <path d="m5 12.5 4.5 4.5L19 8" />
          </svg>
        </button>
      </div>

      <div class="sec">统计与隐私</div>
      <div class="group">
        <button class="opt" :class="{ on: statsEnabled }" @click="setStatsEnabled(!statsEnabled)">
          <span>听歌统计<small class="opt-sub">记录收听时长与次数,用于统计页榜单;数据仅保存在本机,绝不联网上传。关闭后停止统计并隐藏统计页</small></span>
          <svg v-if="statsEnabled" class="check" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
            <path d="m5 12.5 4.5 4.5L19 8" />
          </svg>
        </button>
        <button class="opt" :class="{ armed: clearArmed }" @click="clearStatsConfirmed">
          <span>清除统计数据<small class="opt-sub">{{ clearArmed ? "再点一次确认清除,此操作不可恢复" : "删除本机全部听歌统计,不可恢复" }}</small></span>
        </button>
      </div>

      <div class="sec">皮肤</div>
      <div class="group">
        <button
          v-for="s in skinOptions"
          :key="s.value"
          class="opt"
          :class="{ on: ui.skin === s.value }"
          @click="setSkin(s.value)"
        >
          <span>{{ s.label }}</span>
          <svg v-if="ui.skin === s.value" class="check" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
            <path d="m5 12.5 4.5 4.5L19 8" />
          </svg>
        </button>
      </div>

      <div class="sec">外观</div>
      <div class="group">
        <button
          v-for="opt in themeOptions"
          :key="opt.value"
          class="opt"
          :class="{ on: ui.themeMode === opt.value }"
          @click="setThemeMode(opt.value)"
        >
          <span>{{ opt.label }}</span>
          <svg v-if="ui.themeMode === opt.value" class="check" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
            <path d="m5 12.5 4.5 4.5L19 8" />
          </svg>
        </button>
        <button class="opt" :class="{ on: lyricsOn }" @click="toggleLyrics">
          <span>桌面歌词<small class="opt-sub">桌面置顶小窗:逐行滚动歌词 + 播放/切歌/音量控制,透明度与位置可调,关闭主窗口仍在</small></span>
          <svg v-if="lyricsOn" class="check" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
            <path d="m5 12.5 4.5 4.5L19 8" />
          </svg>
        </button>
      </div>

      <div class="sec">导入</div>
      <div class="group">
        <div class="row">
          <span class="row-label">导入文件夹</span>
          <div class="btns">
            <button class="mini" @click="addFolder">添加音乐文件夹</button>
            <button class="mini" @click="openImportDir">打开</button>
          </div>
        </div>
        <p class="dir" :title="lib.importDir">{{ lib.importDir }}</p>
        <p class="hint">{{ FORMAT_HINT }}</p>
        <div class="row">
          <span class="row-label">导入音乐文件</span>
          <div class="btns">
            <button class="mini" @click="importMusicPick">选择文件</button>
          </div>
        </div>
        <p class="hint">支持多选;导入后自动通过网易云公开接口匹配封面与歌词。APE / WMA 等非常规格式自动经内置 ffmpeg 转为无损 FLAC,原文件保持不动。</p>
        <div class="row">
          <span class="row-label">TMC 音乐包</span>
          <div class="btns">
            <button class="mini" @click="importTmcPick">导入 TMC / TMCL</button>
          </div>
        </div>
        <p class="hint">{{ TMC_HINT }}</p>
      </div>

      <div class="sec">资料库</div>
      <div class="group">
        <div class="row">
          <span class="row-label">{{ lib.tracks.length }} 首歌曲 · {{ lib.folders.length }} 个文件夹</span>
          <button class="mini" :disabled="lib.scanning" @click="rescan">
            {{ lib.scanning ? `扫描中 ${lib.scanCurrent}/${lib.scanTotal}` : "重新扫描" }}
          </button>
        </div>
      </div>

      <div class="sec">格式关联</div>
      <div class="group">
        <button v-for="a in assocs" :key="a.ext" class="opt" @click="toggleAssoc(a)">
          <span>{{ a.label }} <span class="ext">.{{ a.ext }}</span></span>
          <svg v-if="a.registered" class="check" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
            <path d="m5 12.5 4.5 4.5L19 8" />
          </svg>
        </button>
        <p class="hint">{{ ASSOC_HINT }}</p>
      </div>
    </div>
  </div>
</template>

<style scoped>
.view {
  height: 100%;
  overflow-y: auto;
  padding: 22px 28px 40px;
}
/* Apple 设置式窄列:长文可读宽度,分组卡片靠左成栏 */
.col {
  max-width: 720px;
  margin-top: 14px;
}

.sec {
  font-size: 11px;
  font-weight: 700;
  color: var(--text-3);
  letter-spacing: 0.4px;
  padding: 14px 2px 6px;
}
.group {
  border: 1px solid var(--hairline);
  border-radius: 12px;
  padding: 4px;
  background: var(--bg-2);
}
.opt {
  display: flex;
  align-items: center;
  width: 100%;
  gap: 10px;
  padding: 8px 12px;
  border-radius: 8px;
  font-size: 13px;
  color: var(--text-2);
  text-align: left;
  transition: background 0.2s var(--ease-out-soft), color 0.2s ease;
}
.opt:hover {
  background: var(--hover);
  color: var(--text);
}
.opt.on {
  color: var(--text);
}
.opt :deep(.opt-sub) {
  display: block;
  font-size: 11px;
  font-weight: 400;
  color: var(--text-3);
  margin-top: 1px;
}
.opt .check {
  width: 14px;
  height: 14px;
  margin-left: auto;
  flex: none;
  color: var(--text-2);
}
/* 清除统计的两段确认态:文字转警示色,提示「再点一次才执行」 */
.opt.armed {
  color: var(--danger);
}
.opt.armed :deep(.opt-sub) {
  color: var(--danger);
}

.row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 8px 12px;
}
.btns {
  display: flex;
  gap: 6px;
  flex: none;
}
.row-label {
  font-size: 13px;
  color: var(--text);
  min-width: 0;
}
.mini {
  flex: none;
  font-size: 12px;
  font-weight: 600;
  color: var(--text);
  background: var(--pill-bg);
  border-radius: 999px;
  padding: 5px 14px;
  transition: background 0.2s ease, transform 0.35s var(--ease-spring);
}
.mini:hover {
  background: var(--hover);
}
.mini:disabled {
  opacity: 0.5;
  cursor: default;
}

.dir {
  font-size: 11.5px;
  color: var(--text-3);
  padding: 0 12px 2px;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  direction: rtl;
  text-align: left;
}
.hint {
  font-size: 12px;
  color: var(--text-2);
  padding: 2px 12px 10px;
  line-height: 1.6;
}
.ext {
  color: var(--text-3);
  font-size: 11.5px;
}
</style>
