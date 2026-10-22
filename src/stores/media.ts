import { watch } from "vue";
import { emit, listen } from "@tauri-apps/api/event";
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

/**
 * Windows 系统媒体控制(SMTC)、托盘迷你面板的状态桥:
 * 播放状态的唯一拥有者是前端,这里把 曲名/歌手/封面/状态/进度 推给后端
 * (进度节流到约 1 次/秒),并把 托盘面板与系统控制中心 的按钮事件接回播放器。
 * 托盘面板是独立 WebView:状态经 media://state 全局广播,按钮经 media://action 回传。
 */

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

export function initMediaBridge(): void {
  if (bridgeInited) return; // 与 initTheme/initSkin 同理:重挂载/HMR 时监听器不能叠加
  bridgeInited = true;
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
  });

  // 托盘面板打开时主动索要一次当前状态(事件是增量的,面板错过历史)
  void listen("media://state-request", () => {
    broadcast();
    void broadcastLyrics();
  });

  // 曲目切换 → 元数据与封面(封面缓存是本地绝对路径,Rust 侧解码缩略图)
  let lastTrackId = "";
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
  );

  // 在线封面异步到达:刷新 SMTC 元数据与托盘面板,让封面"先占位后点亮"
  watch(
    () => (current.value ? onlineCovers.get(current.value.id) : undefined),
    () => {
      const t = current.value;
      if (!t) return;
      void api.smtcSetTrack(t.title, t.artist, coverPathOf(t), t.duration);
      broadcast();
    },
  );

  // 播放/暂停状态
  watch(
    () => player.playing,
    (playing) => {
      void api.smtcSetStatus(playing);
      broadcast();
    },
  );

  // 进度:节流到约 1 次/秒(SMTC 时间轴、托盘进度都不需要更高频)
  let lastSent = -1;
  watch(() => player.position, (pos) => {
    if (Math.abs(pos - lastSent) >= 1) {
      lastSent = pos;
      void api.smtcSetPosition(pos, player.duration);
      broadcast();
    }
  });
}
