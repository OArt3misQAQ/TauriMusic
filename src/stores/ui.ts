import { ref, reactive, watchEffect } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { StorageKeys, storageGetString, storageRemove, storageSet } from "./storage";

export type ViewName =
  | "home"
  | "albums"
  | "album"
  | "artists"
  | "songs"
  | "search"
  | "playlists"
  | "playlist"
  | "stats"
  | "trash";
/** 皮肤模式:跟随系统 / 浅色 / 黑色 */
export type ThemeMode = "system" | "light" | "dark";
/** 界面材质皮肤:液态玻璃(默认)/ 纸质 */
export type Skin = "glass" | "paper";

interface UiState {
  view: ViewName;
  /** 当前打开的专辑 key */
  albumKey: string | null;
  /** 当前打开的艺人名 */
  artist: string | null;
  /** 当前打开的播放列表 id */
  playlistId: string | null;
  /** 搜索关键词 */
  search: string;
  /** 解析后的深色状态(供玻璃令牌等使用) */
  dark: boolean;
  /** 皮肤模式选择 */
  themeMode: ThemeMode;
  /** 界面材质皮肤:液态玻璃 / 纸质 */
  skin: Skin;
  /** 全屏播放页(含歌词)是否打开 */
  nowPlayingOpen: boolean;
  /** 首次启动使用声明 */
  welcomeOpen: boolean;
  /** 设置弹窗 */
  settingsOpen: boolean;
  /** 关于弹窗 */
  aboutOpen: boolean;
}

export const ui = reactive<UiState>({
  view: "home",
  albumKey: null,
  artist: null,
  playlistId: null,
  search: "",
  dark: false,
  themeMode: "system",
  skin: "glass",
  nowPlayingOpen: false,
  welcomeOpen: false,
  settingsOpen: false,
  aboutOpen: false,
});

const systemDark = ref(false);

/** 首次启动:未同意过使用声明时弹出 */
export function initWelcome(): void {
  ui.welcomeOpen = storageGetString(StorageKeys.welcome, "") !== "1";
}

/** 同意使用声明并记住,不再弹出 */
export function acceptWelcome(): void {
  storageSet(StorageKeys.welcome, "1");
  ui.welcomeOpen = false;
}

function prefersDark(): boolean {
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

export function go(view: ViewName): void {
  ui.view = view;
  if (view !== "search") ui.search = "";
}

export function openAlbum(key: string): void {
  ui.albumKey = key;
  ui.search = ""; // 侧栏搜索框残留旧关键词会立刻把详情页拽回搜索页
  ui.view = "album";
}

export function openArtist(name: string): void {
  ui.artist = name;
  ui.search = "";
  ui.view = "artists";
}

export function openPlaylist(id: string): void {
  ui.playlistId = id;
  ui.search = "";
  ui.view = "playlist";
}

export function setThemeMode(mode: ThemeMode): void {
  ui.themeMode = mode;
  storageSet(StorageKeys.theme, mode);
}

let themeInited = false;

export function initTheme(): void {
  if (themeInited) return; // HMR/重复调用防抖,避免监听与 effect 叠加
  themeInited = true;
  const saved = storageGetString(StorageKeys.theme, "");
  if (saved === "light" || saved === "dark" || saved === "system") {
    ui.themeMode = saved;
  } else {
    // 迁移旧版 tm-dark 记忆
    const legacy = storageGetString("tm-dark", "");
    ui.themeMode = legacy === "" ? "system" : legacy === "1" ? "dark" : "light";
    storageRemove("tm-dark");
  }

  systemDark.value = prefersDark();
  window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
    systemDark.value = prefersDark();
  });

  watchEffect(() => {
    ui.dark = ui.themeMode === "dark" || (ui.themeMode === "system" && systemDark.value);
    document.documentElement.classList.toggle("dark", ui.dark);
    // 原生窗口边框/标题区颜色同步;"跟随系统"交还给系统
    try {
      void getCurrentWindow().setTheme(ui.themeMode === "system" ? null : ui.themeMode);
    } catch {
      /* 浏览器调试环境无窗口 */
    }
  });
}

/** 皮肤切换:写入 storage 并同步到根元素 data-skin,驱动 CSS 令牌层 */
export function setSkin(skin: Skin): void {
  ui.skin = skin;
  storageSet(StorageKeys.skin, skin);
}

export function initSkin(): void {
  const saved = storageGetString(StorageKeys.skin, "glass");
  ui.skin = saved === "paper" ? "paper" : "glass";
  watchEffect(() => {
    // 玻璃材质仅在液态玻璃皮肤下挂载(纸质为纸面材质,无玻璃规则)
    document.documentElement.dataset.skin = ui.skin;
    document.documentElement.classList.toggle("glass", ui.skin === "glass");
  });
}
