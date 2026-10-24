import { computed, shallowReactive } from "vue";
import { api, onlineToTrack, type OnlineSource, type OnlineTrack } from "../api";
import { flash, playTrack } from "./player";

/** 搜索状态机:idle 无结果 / loading 请求中 / done 有结果或空 / err 失败 */
export type OnlinePhase = "idle" | "loading" | "done" | "err";

export interface OnlineSourceOption {
  id: OnlineSource;
  label: string;
}

export const SOURCES: OnlineSourceOption[] = [
  { id: "netease", label: "网易云" },
  { id: "qq", label: "QQ 音乐" },
  { id: "kugou", label: "酷狗音乐" },
  { id: "kuwo", label: "酷我音乐" },
];

export function sourceLabelOf(id: OnlineSource): string {
  return SOURCES.find((s) => s.id === id)?.label ?? "";
}

interface OnlineState {
  keyword: string;
  phase: OnlinePhase;
  errMsg: string;
  /** 搜索结果(浅响应:整体替换才触发更新,翻页也是换新数组) */
  results: OnlineTrack[];
  /** 已请求到的页(0 起);-1 表示后面没有了 */
  lastPage: number;
  source: OnlineSource;
  /** 正在保存到本地库的曲目 id */
  savingId: string;
}

/** 在线搜索的会话级状态(不落盘):App.vue 用 <component :is> 切视图会整页卸载重挂,
 *  状态留在组件里,搜完切去专辑页再回来关键词/结果/翻页进度就全清零,故上收到此 */
export const online = shallowReactive<OnlineState>({
  keyword: "",
  phase: "idle",
  errMsg: "",
  results: [],
  lastPage: -1,
  source: "netease",
  savingId: "",
});

/** 当前来源的中文名(加载态与结果头展示用) */
export const sourceLabel = computed(() => sourceLabelOf(online.source));

/** fee 语义与后端 online.rs 对齐:0/8 可免费播放,其余为 VIP/付费/受限 */
export function playable(t: OnlineTrack): boolean {
  return t.fee === 0 || t.fee === 8;
}

/** 切来源:清空结果,避免把 A 平台的曲目混进 B 平台队列 */
export function selectSource(s: OnlineSource): void {
  if (online.source === s) return;
  online.source = s;
  online.phase = "idle";
  online.errMsg = "";
  online.results = [];
  online.lastPage = -1;
}

export async function search(): Promise<void> {
  const kw = online.keyword.trim();
  if (!kw || online.phase === "loading") return;
  const src = online.source;
  online.phase = "loading";
  online.errMsg = "";
  online.results = [];
  online.lastPage = -1;
  try {
    const list = await api.onlineSearch(src, kw, 0);
    // 请求期间切了来源:丢弃旧响应,不能把结果写进新来源的列表
    if (online.source !== src) return;
    online.results = list;
    online.lastPage = 0;
    online.phase = "done";
  } catch (e) {
    if (online.source !== src) return;
    online.phase = "err";
    online.errMsg = e instanceof Error ? e.message : String(e);
  }
}

/** 翻页:同关键词追加下一页;不足一页(30 条)视为末页 */
export async function loadMore(): Promise<void> {
  const kw = online.keyword.trim();
  if (!kw || online.lastPage < 0 || online.phase === "loading") return;
  const src = online.source;
  const prev = online.phase;
  online.phase = "loading";
  try {
    const list = await api.onlineSearch(src, kw, online.lastPage + 1);
    if (online.source !== src) return;
    online.lastPage += 1;
    // 不足一页即到末页;空页也视为没有更多
    if (list.length < 30) online.lastPage = -1;
    const seen = new Set(online.results.map((t) => t.id));
    const fresh = list.filter((t) => !seen.has(t.id));
    // shallowReactive 的数组必须整体替换,原地 push 不触发视图更新
    if (fresh.length > 0) online.results = [...online.results, ...fresh];
    online.phase = prev;
  } catch (e) {
    if (online.source !== src) return;
    flash(e instanceof Error ? e.message : String(e));
    online.phase = "done";
  }
}

/** 点击结果:免费曲目入队播放;VIP/受限给即时反馈 */
export function play(t: OnlineTrack): void {
  if (!playable(t)) {
    const label = sourceLabelOf(t.source);
    flash(t.fee === 1 ? `VIP 曲目,需要${label}会员` : "该曲目需购买或暂无版权");
    return;
  }
  playTrack(
    onlineToTrack(t),
    online.results.filter(playable).map(onlineToTrack),
  );
}

/** 保存到本地库:与播放同一道闸,付费曲目取链就失败,不存在绕过;
 *  savingId 一次只放行一首,并发写库会让扫描与指纹互相打架 */
export async function save(t: OnlineTrack): Promise<void> {
  if (!playable(t) || online.savingId) return;
  online.savingId = t.id;
  try {
    const r = await api.onlineDownload(
      t.source,
      t.id,
      { name: t.name, artist: t.artist, album: t.album, cover: t.cover },
    );
    // 内容指纹判重:同一首反复保存不会在库里堆出两份
    flash(
      r.duplicates > 0 || r.skipped > 0
        ? "这首已经在本地库里"
        : `已存入本地库:${t.name}`,
    );
  } catch (e) {
    flash(e instanceof Error ? e.message : String(e));
  } finally {
    online.savingId = "";
  }
}
