import { computed, shallowReactive } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import type { Track } from "../api";
import { StorageKeys, storageGetString, storageSet } from "./storage";

/** 播放模式:互斥单选,一键循环切换 */
export type PlayMode = "seq" | "loop" | "one" | "shuffle";

export const MODE_ORDER: PlayMode[] = ["seq", "loop", "one", "shuffle"];
export const MODE_LABEL: Record<PlayMode, string> = {
  seq: "顺序播放",
  loop: "列表循环",
  one: "单曲循环",
  shuffle: "随机播放",
};

interface PlayerState {
  queue: Track[];
  index: number;
  playing: boolean;
  position: number;
  duration: number;
  volume: number;
  mode: PlayMode;
}

export const player = shallowReactive<PlayerState>({
  queue: [],
  index: -1,
  playing: false,
  position: 0,
  duration: 0,
  volume: Number(storageGetString(StorageKeys.volume, "1")),
  mode: loadSavedMode(),
});

function loadSavedMode(): PlayMode {
  const saved = storageGetString(StorageKeys.mode, "seq");
  return (MODE_ORDER as string[]).includes(saved) ? (saved as PlayMode) : "seq";
}

/** 切换播放模式并持久化(重启后保持) */
export function setMode(m: PlayMode): void {
  const wasShuffle = player.mode === "shuffle";
  player.mode = m;
  // 每次切入随机都重新洗牌:上轮随机已放过的歌不再被旧洗牌袋记着,
  // 新一轮随机从全队列重新洗起;播放历史一并作废(prev 语义随新轮次走)
  if (m === "shuffle" && !wasShuffle) {
    bag = [];
    playHistory = [];
  }
  storageSet(StorageKeys.mode, m);
}

export const current = computed<Track | null>(() => player.queue[player.index] ?? null);

/** 进度超过该秒数时按"上一曲"先回到开头 */
const PREV_RESTART_THRESHOLD = 3;

let audio: HTMLAudioElement | null = null;
/** 连续加载失败计数,成功播放后清零(防全坏队列无限跳曲) */
let errorStreak = 0;

/** 实际播放过的下标历史:prev() 据此回到"真正播过的那首"(随机模式下尤其重要) */
let playHistory: number[] = [];
/** 洗牌袋:当前队列下标的一个随机排列,播完一轮才重新洗牌(避免有放回抽样的重复感) */
let bag: number[] = [];
/** 袋子对应的队列引用:队列被替换(playTracks/删除清理)时自动重建 */
let bagQueue: Track[] | null = null;

// 开发期 HMR 热替换会丢弃旧模块实例,但旧的 <audio> 元素仍留在内存里继续出声,
// 其 ended→replay 自循环监听也一并存活——不清理的话会积累出
// 「UI 已暂停、喇叭还在放同一首」的孤儿音频。dispose 时统一停掉并断开
if (import.meta.hot) {
  import.meta.hot.dispose(() => {
    if (audio) {
      audio.pause();
      audio.removeAttribute("src");
      audio.load();
    }
  });
}

function pushHistory(idx: number): void {
  if (idx < 0) return;
  playHistory.push(idx);
  if (playHistory.length > 200) playHistory.shift();
}

function rebuildBag(n: number): void {
  bag = Array.from({ length: n }, (_, i) => i);
  for (let i = bag.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [bag[i], bag[j]] = [bag[j], bag[i]];
  }
}

/** 从洗牌袋取下一个下标:整袋播完才重洗,且绝不与当前曲目立即重复 */
function takeShuffleIndex(n: number, current: number): number {
  if (bagQueue !== player.queue) {
    bag = [];
    bagQueue = player.queue;
  }
  for (;;) {
    if (bag.length === 0) rebuildBag(n);
    if (n > 1 && bag[bag.length - 1] === current) {
      // 袋顶恰是当前曲目:与非当前的随机位置交换;若袋里只剩它则重洗并剔除
      if (bag.length > 1) {
        const j = Math.floor(Math.random() * (bag.length - 1));
        [bag[bag.length - 1], bag[j]] = [bag[j], bag[bag.length - 1]];
      } else {
        rebuildBag(n);
        bag = bag.filter((x) => x !== current);
      }
      continue;
    }
    let idx = bag.pop();
    if (idx === undefined) return current; // n<=1 情形,调用方已拦截,兜底
    // 同专辑规避:全库队列按专辑聚簇,均匀随机也会偶发同专辑连播。
    // 候选与当前曲同专辑且袋里还有异专辑候选时,换一个异专辑的上来,
    // 同专辑的压回袋底(很久之后才轮到)
    if (n > 2 && current >= 0 && current < n && bag.length > 0) {
      const curAlbum = player.queue[current]?.album;
      if (curAlbum && player.queue[idx]?.album === curAlbum) {
        const alt = bag.findIndex((b) => player.queue[b]?.album !== curAlbum);
        if (alt >= 0) {
          bag.unshift(idx);
          idx = bag.splice(alt, 1)[0];
        }
      }
    }
    return idx;
  }
}

function ensureAudio(): HTMLAudioElement {
  if (audio) return audio;
  audio = new Audio();
  audio.volume = Math.min(1, Math.max(0, player.volume));

  // 进度采集用 timeupdate 事件:后台媒体任务不受页面可见性节流,
  // 窗口最小化/隐藏到托盘后仍持续派发。旧实现用 rAF,WebView2 在
  // 隐藏窗口停发 rAF,position 冻结 → 托盘/SMTC 全部假死(长跑大坑)
  audio.addEventListener("timeupdate", () => {
    if (audio && !audio.paused) {
      const t = audio.currentTime;
      if (Math.abs(t - player.position) > 0.2) player.position = t;
    }
  });
  audio.addEventListener("play", () => {
    player.playing = true;
    errorStreak = 0;
  });
  audio.addEventListener("pause", () => {
    player.playing = false;
    if (audio) player.position = audio.currentTime;
  });
  audio.addEventListener("ended", () => onEnded());
  audio.addEventListener("loadedmetadata", () => {
    if (audio && isFinite(audio.duration)) player.duration = audio.duration;
  });
  audio.addEventListener("error", () => {
    console.error("播放失败:", audio?.src);
    player.playing = false;
    // 队列里还有别的歌时自动跳下一曲,连续失败达到队列长度则停止,避免死循环
    if (player.queue.length > 1 && errorStreak < player.queue.length) {
      errorStreak += 1;
      next(false);
    }
  });
  return audio;
}

function load(track: Track): void {
  const a = ensureAudio();
  player.position = 0;
  player.duration = track.duration || 0;
  a.src = convertFileSrc(track.path);
  void a.play().catch(() => {});
}

/** 删除曲目后同步清理播放队列:修正 index,正在播的被删则清空播放器释放句柄 */
export function purgeDeleted(ids: string[]): void {
  if (player.queue.length === 0) return;
  const deletedSet = new Set(ids);
  const removedCurrent = !!current.value && deletedSet.has(current.value.id);
  const kept: Track[] = [];
  for (const t of player.queue) {
    if (deletedSet.has(t.id)) continue;
    kept.push(t);
  }
  if (kept.length === player.queue.length) return; // 队列里没有被删的曲子
  if (removedCurrent || kept.length === 0) {
    // 正在播的被删:清空播放器
    const a = ensureAudio();
    a.pause();
    a.removeAttribute("src");
    a.load();
    player.queue = kept;
    player.index = -1;
    player.playing = false;
    player.position = 0;
    player.duration = 0;
    return;
  }
  // 非当前播放:重建队列并尽量落在原来的曲子上
  const before = current.value?.id ?? "";
  player.queue = kept;
  let idx = kept.findIndex((t) => t.id === before);
  if (idx < 0) idx = Math.max(0, Math.min(player.index, kept.length - 1));
  player.index = idx;
}

/** 以一组曲目作为队列播放,start 为起始下标。新队列重置播放历史与洗牌袋 */
export function playTracks(tracks: Track[], start = 0): void {
  if (tracks.length === 0) return;
  player.queue = tracks;
  player.index = Math.max(0, Math.min(start, tracks.length - 1));
  playHistory = [];
  bag = [];
  bagQueue = tracks;
  load(player.queue[player.index]);
}

/** 在上下文列表中播放某一首(上下文缺省时单曲播放) */
export function playTrack(track: Track, context?: Track[]): void {
  const list = context && context.length > 0 ? context : [track];
  const idx = list.findIndex((t) => t.id === track.id);
  playTracks(list, Math.max(0, idx));
}

export function toggle(): void {
  if (!current.value) return;
  const a = ensureAudio();
  if (a.paused) void a.play().catch(() => {});
  else a.pause();
}

function replay(): void {
  const a = ensureAudio();
  a.currentTime = 0;
  void a.play().catch(() => {});
}

function stop(): void {
  const a = ensureAudio();
  a.pause();
  a.currentTime = 0;
}

export function next(manual = true): void {
  const n = player.queue.length;
  if (n === 0) return;
  const mode = player.mode;

  if (mode === "one" && !manual) {
    replay();
    return;
  }
  if (n === 1) {
    if (manual || mode === "loop") replay();
    return;
  }

  let idx: number;
  if (mode === "shuffle") {
    idx = takeShuffleIndex(n, player.index);
  } else {
    idx = player.index + 1;
    if (idx >= n) {
      if (mode === "loop" || manual) idx = 0;
      else {
        stop();
        return;
      }
    }
  }
  pushHistory(player.index);
  player.index = idx;
  load(player.queue[idx]);
}

export function prev(): void {
  const n = player.queue.length;
  if (n === 0) return;
  const a = ensureAudio();
  if (a.currentTime > PREV_RESTART_THRESHOLD) {
    a.currentTime = 0;
    return;
  }
  // 优先回到实际播放过的上一首(随机模式下这是唯一正确的语义);
  // 历史为空时退化为队列顺序的上一首
  let idx = playHistory.pop();
  if (idx === undefined || idx < 0 || idx >= n) {
    idx = player.index - 1;
    if (idx < 0) idx = player.mode === "loop" ? n - 1 : 0;
  }
  player.index = idx;
  load(player.queue[idx]);
}

function onEnded(): void {
  if (player.mode === "one") {
    replay();
    return;
  }
  next(false);
}

export function seek(t: number): void {
  const a = ensureAudio();
  if (isFinite(t) && t >= 0 && (player.duration === 0 || t <= player.duration + 1)) {
    a.currentTime = t;
    player.position = t;
  }
}

let volSaveTimer = 0;

export function setVolume(v: number): void {
  const clamped = Math.min(1, Math.max(0, v));
  player.volume = clamped;
  // 拖动过程每帧都会调用,落盘做防抖
  window.clearTimeout(volSaveTimer);
  volSaveTimer = window.setTimeout(() => storageSet(StorageKeys.volume, String(clamped)), 300);
  if (audio) audio.volume = clamped;
}

/** 顺序播放 → 列表循环 → 单曲循环 → 随机播放 → 顺序播放(切换即持久化) */
export function cycleMode(): void {
  const i = MODE_ORDER.indexOf(player.mode);
  setMode(MODE_ORDER[(i + 1) % MODE_ORDER.length]);
}

/** 播放队列面板:按"真实播放顺序"列出本轮循环接下来的曲目。
 *  随机模式 = 洗牌袋的剩余顺序(整袋播完前不会重复);
 *  顺序播放不回卷;循环模式转一圈。袋子只随 queue/index/mode 变动,
 *  面板的 computed 依赖这三者即可与真实播放顺序保持一致 */
export function upcomingTracks(): { track: Track; queueIndex: number }[] {
  const q = player.queue;
  const n = q.length;
  if (n === 0) return [];
  const cur = player.index;
  if (player.mode === "shuffle") {
    if (bagQueue !== q || bag.length === 0) {
      // 袋子还没建(队列刚替换/还没播过下一曲):真实顺序未定,先按队列顺序展示
      return q.map((track, i) => ({ track, queueIndex: i })).filter((x) => x.queueIndex !== cur);
    }
    return [...bag]
      .reverse()
      .filter((i) => i !== cur && i >= 0 && i < n)
      .map((i) => ({ track: q[i], queueIndex: i }));
  }
  if (cur < 0) {
    return q.map((track, i) => ({ track, queueIndex: i }));
  }
  const out: { track: Track; queueIndex: number }[] = [];
  for (let k = 1; k < n; k++) {
    const i = (cur + k) % n;
    if (player.mode === "seq" && i < cur) break; // 顺序播放播到队尾即停,不回卷
    out.push({ track: q[i], queueIndex: i });
  }
  return out;
}

/** 跳到队列中的某一首(队列面板点选)。计入播放历史,"上一曲"语义保持正确 */
export function jumpToQueueIndex(i: number): void {
  const q = player.queue;
  if (i < 0 || i >= q.length || i === player.index) return;
  pushHistory(player.index);
  // 随机模式:被点到的曲子必须从洗牌袋剔除,否则本轮还会再抽到它
  if (player.mode === "shuffle" && bagQueue === q) {
    bag = bag.filter((x) => x !== i);
  }
  player.index = i;
  load(q[i]);
}

/** 从队列中移除某一首(不删文件)。正在播的不可移除;
 *  洗牌袋必须同步修正——否则袋里留着失效下标,后续随机会抽到空曲 */
export function removeFromQueue(i: number): void {
  const q = player.queue;
  const n = q.length;
  if (i < 0 || i >= n || n <= 1 || i === player.index) return;
  const oldRef = q;
  player.queue = q.filter((_, k) => k !== i);
  if (i < player.index) player.index -= 1;
  // 历史里的下标同步左移,并剔除被移除曲自身,否则"上一曲"会回到错误的歌
  playHistory = playHistory
    .filter((h) => h !== i)
    .map((h) => (h > i ? h - 1 : h));
  if (bagQueue === oldRef) {
    // 袋子还对应旧队列:剔除被移除的下标,并把大于它的整体左移一号,延续本轮
    bag = bag.filter((x) => x !== i).map((x) => (x > i ? x - 1 : x));
    bagQueue = player.queue;
  }
}

export function fmtTime(s: number): string {
  if (!isFinite(s) || s < 0) s = 0;
  const m = Math.floor(s / 60);
  const sec = Math.floor(s % 60);
  return `${m}:${String(sec).padStart(2, "0")}`;
}
