<script setup lang="ts">
import { ref } from "vue";
import { api, onlineToTrack, type OnlineSource, type OnlineTrack } from "../api";
import { flash, fmtTime, playTrack } from "../stores/player";

/** 搜索状态机:idle 无结果 / loading 请求中 / done 有结果或空 / err 失败 */
type Phase = "idle" | "loading" | "done" | "err";
const phase = ref<Phase>("idle");
const errMsg = ref("");
const keyword = ref("");
/** 已请求到的页(0 起);-1 表示后面没有了 */
const lastPage = ref(-1);
const results = ref<OnlineTrack[]>([]);

const SOURCES: { id: OnlineSource; label: string }[] = [
  { id: "netease", label: "网易云" },
  { id: "qq", label: "QQ 音乐" },
  { id: "kugou", label: "酷狗音乐" },
  { id: "kuwo", label: "酷我音乐" },
];
const source = ref<OnlineSource>("netease");

/** 切来源:清空结果,避免把 A 平台的曲目混进 B 平台队列 */
function selectSource(s: OnlineSource): void {
  if (source.value === s) return;
  source.value = s;
  phase.value = "idle";
  errMsg.value = "";
  results.value = [];
  lastPage.value = -1;
}

/** fee 语义与后端 online.rs 对齐:0/8 可免费播放,其余为 VIP/付费/受限 */
function playable(t: OnlineTrack): boolean {
  return t.fee === 0 || t.fee === 8;
}

async function search(): Promise<void> {
  const kw = keyword.value.trim();
  if (!kw || phase.value === "loading") return;
  const src = source.value;
  phase.value = "loading";
  errMsg.value = "";
  results.value = [];
  lastPage.value = -1;
  try {
    const list = await api.onlineSearch(src, kw, 0);
    // 请求期间切了来源:丢弃旧响应,不能把结果写进新来源的列表
    if (source.value !== src) return;
    results.value = list;
    lastPage.value = 0;
    phase.value = "done";
  } catch (e) {
    if (source.value !== src) return;
    phase.value = "err";
    errMsg.value = e instanceof Error ? e.message : String(e);
  }
}

/** 翻页:追加下一页结果(同关键词) */
async function loadMore(): Promise<void> {
  const kw = keyword.value.trim();
  if (!kw || lastPage.value < 0 || phase.value === "loading") return;
  const src = source.value;
  const prev = phase.value;
  phase.value = "loading";
  try {
    const list = await api.onlineSearch(src, kw, lastPage.value + 1);
    if (source.value !== src) return;
    lastPage.value += 1;
    // 不足一页即到末页;空页也视为没有更多
    if (list.length < 30) lastPage.value = -1;
    const seen = new Set(results.value.map((t) => t.id));
    for (const t of list) if (!seen.has(t.id)) results.value.push(t);
    phase.value = prev;
  } catch (e) {
    if (source.value !== src) return;
    flash(e instanceof Error ? e.message : String(e));
    phase.value = "done";
  }
}

/** 点击结果:免费曲目入队播放;VIP/受限给即时反馈 */
function play(t: OnlineTrack): void {
  if (!playable(t)) {
    const label = SOURCES.find((s) => s.id === t.source)?.label ?? "该平台";
    flash(t.fee === 1 ? `VIP 曲目,需要${label}会员` : "该曲目需购买或暂无版权");
    return;
  }
  playTrack(
    onlineToTrack(t),
    results.value.filter(playable).map(onlineToTrack),
  );
}
</script>

<template>
  <div class="view">
    <div class="ov-head">
      <h1>在线音乐</h1>
      <p class="sub">搜索并试听各平台免费曲目 · 仅供试听,支持正版</p>
      <div class="sources" role="tablist" aria-label="在线音源">
        <button
          v-for="s in SOURCES"
          :key="s.id"
          type="button"
          role="tab"
          :aria-selected="source === s.id"
          class="src"
          :class="{ on: source === s.id }"
          @click="selectSource(s.id)"
        >
          {{ s.label }}
        </button>
      </div>
      <form class="ov-search" @submit.prevent="search">
        <svg class="s-ico" viewBox="0 0 24 24">
          <circle cx="11" cy="11" r="6.5" fill="none" stroke="currentColor" stroke-width="1.8" />
          <path d="m16 16 4.5 4.5" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
        </svg>
        <input
          v-model="keyword"
          type="search"
          enterkeyhint="search"
          placeholder="歌曲 / 艺人 / 专辑"
          @keydown.enter.prevent="search"
        />
        <button v-if="keyword" class="clear" type="button" title="清空" @click="keyword = ''">
          <svg viewBox="0 0 24 24"><path d="m7 7 10 10M17 7 7 17" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" /></svg>
        </button>
        <button class="go" type="submit" :disabled="!keyword.trim() || phase === 'loading'">
          {{ phase === "loading" && results.length === 0 ? "搜索中…" : "搜索" }}
        </button>
      </form>
    </div>

    <div v-if="phase === 'err'" class="empty">
      <h2>搜索失败</h2>
      <p>{{ errMsg }}</p>
    </div>

    <div v-else-if="phase === 'done' && results.length === 0" class="empty">
      <h2>没有找到“{{ keyword }}”相关曲目</h2>
      <p>试试其他关键词</p>
    </div>

    <div v-else-if="results.length" class="ov-list">
      <div v-for="(t, i) in results" :key="t.id" class="ov-row" :class="{ dim: !playable(t) }">
        <button class="ov-main" @click="play(t)">
          <span class="no">{{ i + 1 }}</span>
          <span class="meta">
            <span class="t">{{ t.name }}</span>
            <span class="a">{{ t.artist }}<template v-if="t.album"> · {{ t.album }}</template></span>
          </span>
          <span v-if="!playable(t)" class="vip">VIP</span>
          <span class="dur">{{ fmtTime(t.duration) }}</span>
        </button>
      </div>
      <button v-if="lastPage >= 0" class="more" :disabled="phase === 'loading'" @click="loadMore">
        {{ phase === "loading" ? "加载中…" : "加载更多" }}
      </button>
      <p v-else-if="results.length >= 30" class="end-hint">没有更多了</p>
    </div>

    <div v-else class="empty">
      <h2>搜索在线音乐</h2>
      <p>输入关键词,按回车或点击搜索</p>
    </div>
  </div>
</template>

<style scoped>
.ov-head {
  margin-bottom: 18px;
}
.ov-head h1 {
  font-size: 22px;
  font-weight: 800;
}
.sub {
  font-size: 12px;
  color: var(--text-3);
  margin-top: 3px;
}
.sources {
  display: flex;
  gap: 4px;
  margin-top: 12px;
}
.src {
  height: 28px;
  padding: 0 14px;
  border-radius: 999px;
  background: var(--pill-bg);
  border: 1px solid var(--hairline);
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text-2);
  transition:
    background 0.18s ease,
    color 0.18s ease,
    border-color 0.18s ease;
}
.src:hover {
  color: var(--text);
}
.src.on {
  background: var(--accent);
  border-color: var(--accent);
  color: #fff;
}
.ov-search {
  margin-top: 14px;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 8px 0 12px;
  height: 42px;
  border-radius: 12px;
  background: var(--pill-bg);
  border: 1px solid var(--hairline);
}
.ov-search:focus-within {
  border-color: var(--accent);
}
.s-ico {
  width: 17px;
  height: 17px;
  color: var(--text-3);
  flex: none;
}
.ov-search input {
  flex: 1;
  min-width: 0;
  height: 100%;
  font-size: 14px;
  background: transparent;
}
.ov-search input::placeholder {
  color: var(--text-3);
}
.clear {
  flex: none;
  width: 26px;
  height: 26px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-3);
}
.clear:hover {
  background: var(--hover);
  color: var(--text);
}
.clear svg {
  width: 13px;
  height: 13px;
}
.go {
  flex: none;
  height: 30px;
  padding: 0 16px;
  border-radius: 999px;
  background: var(--accent);
  color: #fff;
  font-size: 13px;
  font-weight: 600;
}
.go:disabled {
  opacity: 0.45;
  cursor: default;
}
.go:hover:not(:disabled) {
  background: var(--accent-hover);
}

.ov-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.ov-row {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 9px 10px;
  border-radius: 9px;
  transition: background 0.2s var(--ease-out-soft);
}
.ov-row:hover {
  background: var(--hover);
}
.ov-row.dim {
  opacity: 0.55;
}
.ov-main {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 12px;
  text-align: left;
}
.ov-main:active {
  transform: scale(0.995);
}
.no {
  width: 22px;
  flex: none;
  text-align: center;
  font-size: 12px;
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
}
.meta {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.t {
  font-size: 13.5px;
  font-weight: 500;
  color: var(--text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.a {
  font-size: 12px;
  color: var(--text-2);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.vip {
  flex: none;
  font-size: 9.5px;
  font-weight: 700;
  color: #c8a24b;
  border: 1px solid #c8a24b;
  border-radius: 4px;
  padding: 0 4px;
  line-height: 15px;
}
.dur {
  flex: none;
  width: 38px;
  text-align: right;
  font-size: 12px;
  color: var(--text-2);
  font-variant-numeric: tabular-nums;
}
.more {
  align-self: center;
  margin: 14px 0 4px;
  padding: 7px 26px;
  border-radius: 999px;
  background: var(--pill-bg);
  border: 1px solid var(--hairline);
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text-2);
}
.more:hover:not(:disabled) {
  background: var(--hover);
  color: var(--text);
}
.end-hint {
  align-self: center;
  margin: 14px 0 4px;
  font-size: 11.5px;
  color: var(--text-3);
}
</style>
