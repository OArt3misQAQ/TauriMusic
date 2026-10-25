import { ref, watch } from "vue";
import { emit, listen } from "@tauri-apps/api/event";
import { convertFileSrc } from "@tauri-apps/api/core";
import {
  player,
  current,
  toggle,
  next,
  prev,
  seek,
  setVolume,
  coverPathOf,
  onlineCovers,
} from "./player";
import { api, type LyricLine } from "../api";
import { isMobile } from "../platform";

/**
 * Windows 系统媒体控制(SMTC)、托盘迷你面板的状态桥:
 * 播放状态的唯一拥有者是前端,这里把 曲名/歌手/封面/状态/进度 推给后端
 * (进度节流到约 1 次/秒),并把 托盘面板与系统控制中心 的按钮事件接回播放器。
 * 托盘面板是独立 WebView:状态经 media://state 全局广播,按钮经 media://action 回传。
 */

/** 桌面歌词小窗开关(持久化):窗口显示/隐藏由 Rust 命令执行;
 *  小窗自己的关闭按钮也写同一份 localStorage,storage 事件跨窗口同步回显 */
export const lyricsOn = ref(localStorage.getItem("tm-lyrics-on") === "1");

/** 切换桌面歌词小窗。先执行命令成功后才提交状态:失败时开关与真实窗口脱节,
 *  下次启动还会按 localStorage 把小窗拉起来。返回错误文案供调用方提示 */
export async function toggleLyricsWindow(): Promise<string | null> {
  const next = !lyricsOn.value;
  try {
    await (next ? api.showLyricsWindow() : api.hideLyricsWindow());
    lyricsOn.value = next;
    localStorage.setItem("tm-lyrics-on", next ? "1" : "0");
    return null;
  } catch (err) {
    console.error("切换歌词小窗失败", err);
    return String(err);
  }
}

function onLyricsStorage(e: StorageEvent): void {
  if (e.key === "tm-lyrics-on") lyricsOn.value = e.newValue === "1";
}
// 设置页/播放页/小窗自身可能同时开着:开关写的是各自文档的 localStorage,
// 靠 storage 事件互相同步,否则开着开关的一方与真实窗口状态不一致
window.addEventListener("storage", onLyricsStorage);

/** 把当前播放状态广播给托盘面板/歌词小窗(以及未来任何需要它的窗口) */
function broadcast(): void {
  const t = current.value;
  void emit("media://state", {
    title: t?.title ?? "",
    artist: t?.artist ?? "",
    // 在线曲目经 coverPathOf 兜底到在线封面缓存(本地路径,托盘 convertFileSrc 可渲染)
    cover: coverPathOf(t),
    playing: player.playing,
    position: player.position,
    duration: player.duration,
    volume: player.volume,
  });
}

/** 歌词小窗的歌词数据:切歌时拉取并广播(音量/状态走 media://state,歌词单独走) */
interface WidgetLyrics {
  title: string;
  artist: string;
  synced: LyricLine[];
  plain: string | null;
}

let widgetLyrics: WidgetLyrics = { title: "", artist: "", synced: [], plain: null };

async function broadcastLyrics(): Promise<void> {
  const t = current.value;
  if (!t) {
    widgetLyrics = { title: "", artist: "", synced: [], plain: null };
    void emit("media://lyrics", widgetLyrics);
    return;
  }
  try {
    const payload = await api.getLyrics(t.id);
    // 快速切歌时旧请求可能后返回:不校验就会把新歌的歌词窗覆盖成旧歌的词
    if (current.value?.id !== t.id) return;
    widgetLyrics = {
      title: t.title,
      artist: t.artist,
      synced: payload?.synced ?? [],
      plain: payload?.plain ?? null,
    };
  } catch {
    if (current.value?.id !== t.id) return;
    widgetLyrics = { title: t.title, artist: t.artist, synced: [], plain: null };
  }
  void emit("media://lyrics", widgetLyrics);
}

let bridgeInited = false;
/** HMR dispose 用:本模块注册的 Tauri 事件反注册与 watch 停止函数 */
let unlisteners: (() => void)[] = [];
let stopWatches: (() => void)[] = [];

export function initMediaBridge(): void {
  if (bridgeInited) return; // 与 initTheme/initSkin 同理:重挂载/HMR 时监听器不能叠加
  bridgeInited = true;
  // 手机没有托盘/桌面歌词窗口:SMTC 推送、media:// 广播和歌词窗口的额外取词
  // 全是白跑,直接只留系统媒体卡片(WebView 支持时)
  if (isMobile) {
    initAndroidMediaSession(stopWatches);
    return;
  }
  // listen 返回的是 Promise,反注册函数到手后归集,供 HMR dispose 统一摘除
  const keepUnlisten = (un: () => void) => unlisteners.push(un);
  // 托盘面板/系统控制中心的按钮 → 播放器动作
  void listen<string>("media://action", (e) => {
    // 带参动作:进度拖拽 "seek:<秒>"(托盘进度条可拖)
    if (e.payload.startsWith("seek:")) {
      const t = Number(e.payload.slice(5));
      if (Number.isFinite(t)) {
        seek(t);
        // position 的广播有 ≥1s 的节流,seek 幅度小或暂停状态时不会触发,
        // 托盘会一直停在拖拽预览值——动作执行完立即广播一次兜底
        broadcast();
      }
      return;
    }
    // 带参动作:歌词小窗音量 "volume:<0~1>"
    if (e.payload.startsWith("volume:")) {
      const v = Number(e.payload.slice(7));
      if (Number.isFinite(v)) setVolume(v);
      return;
    }
    switch (e.payload) {
      case "toggle":
        toggle();
        break;
      case "play":
        if (!player.playing) toggle();
        break;
      case "pause":
        if (player.playing) toggle();
        break;
      case "next":
        next();
        break;
      case "prev":
        prev();
        break;
    }
  }).then(keepUnlisten);

  // 托盘面板打开时主动索要一次当前状态(事件是增量的,面板错过历史)
  void listen("media://state-request", () => {
    broadcast();
    void broadcastLyrics();
  }).then(keepUnlisten);

  // 曲目切换 → 元数据与封面(封面缓存是本地绝对路径,Rust 侧解码缩略图)
  let lastTrackId = "";
  stopWatches.push(
    watch(
      () => current.value?.id ?? "",
      (id) => {
        if (!id || id === lastTrackId) return;
        lastTrackId = id;
        const t = current.value;
        if (!t) return;
        void api.smtcSetTrack(t.title, t.artist, coverPathOf(t), t.duration);
        broadcast();
        void broadcastLyrics();
      },
    ),
  );

  // 在线封面异步到达:刷新 SMTC 元数据与托盘面板,让封面"先占位后点亮"
  stopWatches.push(
    watch(
      () => (current.value ? onlineCovers.get(current.value.id) : undefined),
      () => {
        const t = current.value;
        if (!t) return;
        void api.smtcSetTrack(t.title, t.artist, coverPathOf(t), t.duration);
        broadcast();
      },
    ),
  );

  // 播放/暂停状态
  stopWatches.push(
    watch(
      () => player.playing,
      (playing) => {
        void api.smtcSetStatus(playing);
        broadcast();
      },
    ),
  );

  // 进度:节流到约 1 次/秒(SMTC 时间轴、托盘进度都不需要更高频)
  let lastSent = -1;
  stopWatches.push(
    watch(
      () => player.position,
      (pos) => {
        if (Math.abs(pos - lastSent) >= 1) {
          lastSent = pos;
          void api.smtcSetPosition(pos, player.duration);
          broadcast();
        }
      },
    ),
  );

}

/** 移动端媒体卡片:元数据与播放状态随播放器变化,控制按钮接回既有动作 */
function initAndroidMediaSession(stopWatches: (() => void)[]): void {
  if (!("mediaSession" in navigator)) return;
  const session = navigator.mediaSession;
  const bind = (action: MediaSessionAction, handler: MediaSessionActionHandler): void => {
    // 不支持的动作在部分 WebView 上会抛异常,逐个兜住
    try {
      session.setActionHandler(action, handler);
    } catch {
      /* 该动作不被当前 WebView 支持 */
    }
  };

  bind("play", () => {
    if (!player.playing) toggle();
  });
  bind("pause", () => {
    if (player.playing) toggle();
  });
  bind("nexttrack", () => next());
  bind("previoustrack", () => prev());
  bind("seekto", (details) => {
    if (typeof details.seekTime === "number") seek(details.seekTime);
  });

  const update = (): void => {
    const t = current.value;
    if (!t) {
      session.metadata = null;
      session.playbackState = "none";
      return;
    }
    const cover = coverPathOf(t);
    try {
      session.metadata = new MediaMetadata({
        title: t.title,
        artist: t.artist,
        album: t.album,
        artwork: cover ? [{ src: convertFileSrc(cover) }] : [],
      });
    } catch {
      /* 个别 WebView 无 MediaMetadata 构造器:不显示元数据即可 */
    }
    session.playbackState = player.playing ? "playing" : "paused";
  };

  stopWatches.push(
    watch(() => `${current.value?.id ?? ""}|${player.playing}`, update, { immediate: true }),
  );
  // 在线封面异步落盘后补一次,通知栏封面从占位变成真实图
  stopWatches.push(
    watch(
      () => (current.value ? onlineCovers.get(current.value.id) : undefined),
      update,
    ),
  );
}

// 开发期 HMR 热替换后,旧模块的 Tauri 事件监听与 watch 仍存活:
// 同一次切歌/动作会被新旧两座桥各执行一遍(SMTC 双写、托盘双响应)。
// 清理手段与 player.ts 的 dispose 惯例同理
if (import.meta.hot) {
  import.meta.hot.dispose(() => {
    for (const un of unlisteners) un();
    unlisteners = [];
    for (const stop of stopWatches) stop();
    stopWatches = [];
    window.removeEventListener("storage", onLyricsStorage);
  });
}
