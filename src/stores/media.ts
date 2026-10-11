import { watch } from "vue";
import { listen } from "@tauri-apps/api/event";
import { player, current, toggle, next, prev } from "./player";
import { api } from "../api";

/**
 * Windows 系统媒体控制(SMTC)与托盘菜单的状态桥:
 * 播放状态的唯一拥有者是前端,这里把 曲名/歌手/封面/状态/进度 推给后端
 * (进度节流到约 1 次/秒),并把 托盘菜单与系统控制中心 的按钮事件接回播放器。
 */

export function initMediaBridge(): void {
  // 托盘菜单与系统控制中心的按钮 → 播放器动作
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
    },
  );

  // 播放/暂停状态
  watch(
    () => player.playing,
    (playing) => {
      void api.smtcSetStatus(playing);
    },
  );

  // 进度:节流到约 1 次/秒(SMTC 时间轴与托盘进度文本都不需要更高频)
  let lastSent = -1;
  watch(() => player.position, (pos) => {
    if (Math.abs(pos - lastSent) >= 1) {
      lastSent = pos;
      void api.smtcSetPosition(pos, player.duration);
    }
  });
}
