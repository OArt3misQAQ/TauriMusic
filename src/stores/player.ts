import { computed, reactive, shallowReactive } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import { api, type OnlineSource, type Track } from "../api";
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
  // 每次切入随机都可见地重新洗牌:队列真实重排(正在播的提到队首),
  // 播放历史作废;退回其他模式时保留洗好的顺序继续顺序走
  if (m === "shuffle" && !wasShuffle && player.queue.length > 1) {
    reshuffleQueue();
    queueShuffled = true;
    playHistory = [];
  } else if (m !== "shuffle") {
    queueShuffled = false;
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
/** 当前队列来自哪份列表(引用比对):「随机播放」据此判断是重洗还是换列表 */
let queueSource: Track[] | null = null;
/** 随机模式 = 队列已被可见洗牌(切模式/随机按钮时重排),顺序推进即可 */
let queueShuffled = false;

/** 把当前队列原地洗牌:正在播的曲子提到队首,其余 Fisher-Yates 随机重排 */
function reshuffleQueue(): void {
  const q = [...player.queue];
  // 洗牌起始下标:有正在播的曲子时锁定队首不参与交换;空闲(index 无效)则整条全洗
  let head = 0;
  if (player.index >= 0 && player.index < q.length) {
    q.unshift(q.splice(player.index, 1)[0]);
    player.index = 0;
    head = 1;
  }
  // 交换必须从 head 起:队首一旦参与交换,刚提上来的当前曲会被甩走,
  // current 立刻指向别的歌——歌词页切随机「当前歌被直接切换」的根源
  for (let i = q.length - 1; i > head; i--) {
    const j = head + Math.floor(Math.random() * (i - head + 1));
    [q[i], q[j]] = [q[j], q[i]];
  }
  player.queue = q;
  bag = [];
  bagQueue = q;
}

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
  if (track.online && track.onlineId && track.onlineSource) {
    void loadOnline(track, track.onlineSource, track.onlineId, a);
    return;
  }
  a.src = convertFileSrc(track.path);
  void a.play().catch(() => {});
}

/** 取链竞态令牌:在线取链是异步的,返回前用户可能已切歌,
 *  慢响应若直接写 a.src 会把新歌顶掉——序列号不符即丢弃 */
let loadSeq = 0;

/** 在线曲目加载:懒取直链(进播放队列时不请求,真正 load 时才取,避免链接过期)。
 *  取到的直链缓存回 track.onlineUrl,同一首歌切回来不再二次请求 */
async function loadOnline(
  track: Track,
  onlineSource: OnlineSource,
  onlineId: string,
  a: HTMLAudioElement,
): Promise<void> {
  const seq = ++loadSeq;
  // 封面与取链并行懒取,互不阻塞出声
  void ensureOnlineCover(track);
  // 立刻停掉旧曲出声;直链没到手前保持静默,UI 处于"加载中"态
  a.pause();
  a.removeAttribute("src");
  a.load();
  if (track.onlineUrl) {
    if (seq !== loadSeq) return;
    a.src = track.onlineUrl;
    void a.play().catch(() => {});
    return;
  }
  try {
    const url = await api.onlinePlayUrl(onlineSource, onlineId);
    if (seq !== loadSeq) return;
    track.onlineUrl = url;
    a.src = url;
    void a.play().catch(() => {});
  } catch (e) {
    if (seq !== loadSeq) return;
    // VIP/版权受限或网络失败:提示并按"坏曲目"跳下一曲,
    // 与 audio error 走同一道 errorStreak 闸门,防全坏队列死循环
    flash(e instanceof Error ? e.message : String(e));
    player.playing = false;
    if (player.queue.length > 1 && errorStreak < player.queue.length) {
      errorStreak += 1;
      next(false);
    }
  }
}

/** 在线曲目封面缓存(播放队列 id → 本地缓存路径):reactive Map,
 *  取到即触发依赖它的 computed(播放页封面/氛围背景/SMTC)重新求值 */
export const onlineCovers = reactive(new Map<string, string>());

/** 当前曲目渲染用的封面路径:本地曲目走曲库缓存,在线曲目落在线缓存;
 *  统一出口让播放页/氛围背景/SMTC 桥共享同一份兜底顺序 */
export function coverPathOf(t: Track | null | undefined): string | null {
  if (!t) return null;
  return t.coverLarge ?? t.cover ?? onlineCovers.get(t.id) ?? null;
}

/** 在线曲目封面懒取:开播时拉一次,Rust 端落盘到应用封面缓存后返回本地路径。
 *  失败静默(无图/网络错误都保持 ♪ 占位),同一首不重复请求 */
async function ensureOnlineCover(track: Track): Promise<void> {
  // 酷我搜索接口不提供封面,不必每次开播都空跑一次 IPC
  if (
    !track.onlineSource ||
    track.onlineSource === "kuwo" ||
    !track.onlineId ||
    track.cover ||
    onlineCovers.has(track.id)
  )
    return;
  try {
    const p = await api.onlineCover(
      track.onlineSource,
      track.onlineId,
      track.onlineCover ?? null,
    );
    if (p) onlineCovers.set(track.id, p);
  } catch {
    /* 无图或网络失败:占位符兜底,下次播放会再试 */
  }
}

/** 全局提示条(播放失败等瞬时反馈):几秒后自动消失 */
export const playerFlash = shallowReactive({ text: "", at: 0 });
let flashTimer = 0;

export function flash(text: string): void {
  playerFlash.text = text;
  playerFlash.at = Date.now();
  window.clearTimeout(flashTimer);
  flashTimer = window.setTimeout(() => (playerFlash.text = ""), 3600);
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
    // 队列已空:来源标记必须一并作废,否则之后对同一列表点「随机播放」
    // 会命中"同列表已在队列"分支,对着空队列重洗后直接返回,点了没反应
    queueSource = null;
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
  const q = [...tracks];
  let idx = Math.max(0, Math.min(start, q.length - 1));
  if (player.mode === "shuffle" && q.length > 1) {
    // 随机模式下队列可见洗牌:起始曲提到队首,其余随机重排
    q.unshift(q.splice(idx, 1)[0]);
    for (let i = q.length - 1; i > 1; i--) {
      const j = 1 + Math.floor(Math.random() * i);
      [q[i], q[j]] = [q[j], q[i]];
    }
    idx = 0;
    queueShuffled = true;
  } else {
    queueShuffled = false;
  }
  player.queue = q;
  player.index = idx;
  playHistory = [];
  bag = [];
  bagQueue = q;
  queueSource = tracks;
  load(player.queue[player.index]);
}

/** 在上下文列表中播放某一首(上下文缺省时单曲播放) */
export function playTrack(track: Track, context?: Track[]): void {
  const list = context && context.length > 0 ? context : [track];
  const idx = list.findIndex((t) => t.id === track.id);
  playTracks(list, Math.max(0, idx));
}

function shuffled<T>(arr: T[]): T[] {
  const a = [...arr];
  for (let i = a.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [a[i], a[j]] = [a[j], a[i]];
  }
  return a;
}

/** 随机播放语义(绝不自动出声):
 *  有歌在放/暂停中 → 队列改为 [当前曲, ...打乱的列表],当前歌放完自然流入随机顺序;
 *  没在播 → 只备好洗牌队列,等用户自己按播放,绝不自动开播 */
/** 列表「随机播放」——硬保证:绝不打断当前歌、绝不自动出声。
 *  同一列表已在队列 → 只重洗整条队列,当前曲置顶续播;
 *  其他列表 → 当前曲置顶续播,放完自然流入打乱的新列表;
 *  空闲 → 只备好打乱的队列,等用户自己按播放 */
export function shuffleList(list: Track[]): void {
  if (list.length === 0) return;
  setMode("shuffle");
  const cur = current.value;
  if (queueSource === list) {
    // 同一份列表已在队列:整条重排即可(当前曲已在队首续播)
    reshuffleQueue();
    playHistory = [];
    return;
  }
  if (cur) {
    const rest = shuffled(list.filter((t) => t.id !== cur.id));
    player.queue = [cur, ...rest];
    player.index = 0;
  } else {
    player.queue = shuffled(list);
    player.index = 0;
  }
  playHistory = [];
  bag = [];
  bagQueue = player.queue;
  queueSource = list;
}

export function toggle(): void {
  if (!current.value) return;
  const a = ensureAudio();
  // 队列被重排/模式切换后 current 可能尚无 src(从未 load 过):先加载即播放
  if (!a.src) {
    load(player.queue[player.index] ?? current.value);
    return;
  }
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

  // 随机模式且队列已预洗:顺序推进洗好的顺序(与队列抽屉显示一致),
  // 手动切歌到队尾时回卷,自然播完一轮则停下
  if (mode === "shuffle" && queueShuffled) {
    pushHistory(player.index);
    const nx = player.index + 1;
    if (nx < n) {
      player.index = nx;
      load(player.queue[nx]);
    } else if (manual) {
      player.index = 0;
      load(player.queue[0]);
    } else {
      stop();
    }
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
    // 预洗队列:抽屉按洗好的顺序展示当前曲之后的走向
    if (queueShuffled) {
      return q
        .map((track, i) => ({ track, queueIndex: i }))
        .filter((x) => x.queueIndex > cur);
    }
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
