import { player, toggle, next, prev, seek, setVolume } from "./player";
import { ui } from "./ui";
import { ctx } from "./context";
import { exportState, importPreview } from "./library";
import { nameDialog, picker } from "./playlists";

const SEEK_STEP = 5; // 秒
const VOL_STEP = 0.05;

let volBeforeMute = 0;
let inited = false;

/** 全局播放快捷键:
 *  空格 播放/暂停 · ←/→ 快退/快进 5s · Ctrl+←/→ 上一首/下一首
 *  ↑/↓ 音量 ±5% · M 静音 · Esc 关闭全屏播放页
 *  焦点在文本输入框/下拉框时不生效;滑杆上空格与音量键仍可用(方向键保留原生调值) */
export function initShortcuts(): void {
  if (inited) return; // HMR 重复挂载防抖
  inited = true;
  window.addEventListener("keydown", onKey);
  // 点击输入框以外的区域时,把残留的输入框焦点释放掉,避免快捷键失效
  document.addEventListener("click", onBlurAway, true);
}

// 开发期 HMR 热替换后旧模块的键盘/点击监听仍存活:
// 不摘除会一次按键触发两遍快捷键(清理手段与 player.ts 的 dispose 惯例同理)
if (import.meta.hot) {
  import.meta.hot.dispose(() => {
    window.removeEventListener("keydown", onKey);
    document.removeEventListener("click", onBlurAway, true);
  });
}

function onBlurAway(e: MouseEvent): void {
  const t = e.target as HTMLElement | null;
  if (t?.closest("input, textarea, select, [contenteditable]")) return;
  const ae = document.activeElement;
  if (ae instanceof HTMLInputElement || ae instanceof HTMLTextAreaElement) ae.blur();
}

/** 文本输入类控件(空格/M 键在这些控件上让位给打字) */
function isTextInput(el: HTMLElement | null): boolean {
  if (!el) return false;
  if (el.isContentEditable) return true;
  const tag = el.tagName;
  if (tag === "TEXTAREA" || tag === "SELECT") return true;
  if (tag === "INPUT") {
    const type = (el.getAttribute("type") ?? "text").toLowerCase();
    return !["range", "checkbox", "radio", "button"].includes(type);
  }
  return false;
}

/** 表单控件(方向键在这些控件上保留原生行为,如滑杆调值) */
function isFormControl(el: HTMLElement | null): boolean {
  return !!el?.closest("input, textarea, select, [contenteditable]");
}

function onKey(e: KeyboardEvent): void {
  if (e.isComposing) return;
  const t = e.target as HTMLElement | null;

  if (e.key === "Escape") {
    // 各类弹窗/右键菜单有自己的关闭逻辑,不重复处理;
    // 漏掉任何一种都会导致一次 Esc 同时关掉弹窗和全屏播放页两层
    if (
      ui.welcomeOpen ||
      ui.aboutOpen ||
      ctx.items ||
      picker.trackIds ||
      nameDialog.open ||
      exportState.open ||
      importPreview.open
    ) {
      return;
    }
    if (ui.nowPlayingOpen) {
      e.preventDefault();
      ui.nowPlayingOpen = false;
    }
    return;
  }

  // 右键菜单打开时按键交给菜单
  if (ctx.items) return;

  const isSpace = e.key === " " || e.code === "Space";
  if (isSpace || e.key === "m" || e.key === "M") {
    if (isTextInput(t)) return;
  } else if (isFormControl(t)) {
    return; // 方向键:滑杆/下拉框原生行为
  }

  if (isSpace) {
    e.preventDefault();
    toggle();
  } else if (e.key === "m" || e.key === "M") {
    if (player.volume > 0) {
      volBeforeMute = player.volume;
      setVolume(0);
    } else {
      setVolume(volBeforeMute > 0 ? volBeforeMute : 0.5);
    }
  } else if (e.key === "ArrowLeft") {
    e.preventDefault();
    if (e.ctrlKey) prev();
    else seek(player.position - SEEK_STEP);
  } else if (e.key === "ArrowRight") {
    e.preventDefault();
    if (e.ctrlKey) next();
    else {
      const end = player.duration > 0 ? player.duration : player.position + SEEK_STEP;
      seek(Math.min(player.position + SEEK_STEP, end));
    }
  } else if (e.key === "ArrowUp") {
    e.preventDefault();
    setVolume(player.volume + VOL_STEP);
  } else if (e.key === "ArrowDown") {
    e.preventDefault();
    setVolume(player.volume - VOL_STEP);
  }
}
