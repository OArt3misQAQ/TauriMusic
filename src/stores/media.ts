import { watch } from "vue";
import { emit, listen } from "@tauri-apps/api/event";
import { player, current, toggle, next, prev } from "./player";
import { api } from "../api";

/**
 * Windows 系统媒体控制(SMTC)、托盘迷你面板的状态桥:
 * 播放状态的唯一拥有者是前端,这里把 曲名/歌手/封面/状态/进度 推给后端
 * (进度节流到约 1 次/秒),并把 托盘面板与系统控制中心 的按钮事件接回播放器。
 * 托盘面板是独立 WebView:状态经 media://state 全局广播,按钮经 media://action 回传。
 */

/** 把当前播放状态广播给托盘面板(以及未来任何需要它的窗口) */
function broadcast(): void {
  const t = current.value;
  void emit("media://state", {
    title: t?.title ?? "",
    artist: t?.artist ?? "",
    cover: t?.cover ?? null,
    playing: player.playing,
    position: player.position,
    duration: player.duration,
  });
}

export function initMediaBridge(): void {
  // 托盘面板/系统控制中心的按钮 → 播放器动作
  void listen<string>("media://action", (e) => {
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
  void listen("media://state-request", () => broadcast());

  // 曲目切换 → 元数据与封面(封面缓存是本地绝对路径,Rust 侧解码缩略图)
  let lastTrackId = "";
  watch(
    () => current.value?.id ?? "",
    (id) => {
      if (!id || id === lastTrackId) return;
      lastTrackId = id;
      const t = current.value;
      if (!t) return;
      void api.smtcSetTrack(t.title, t.artist, t.cover ?? null, t.duration);
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
