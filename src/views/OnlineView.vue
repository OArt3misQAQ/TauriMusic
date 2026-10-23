<script setup lang="ts">
import { computed, ref } from "vue";
import { api, onlineToTrack, type OnlineQuality, type OnlineSource, type OnlineTrack } from "../api";
import { flash, fmtTime, onlineQuality, playTrack, setOnlineQuality } from "../stores/player";

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
const sourceLabel = computed(() => SOURCES.find((s) => s.id === source.value)?.label ?? "");

const QUALITIES: { id: OnlineQuality; label: string; note: string }[] = [
  { id: "standard", label: "标准", note: "128k" },
  { id: "high", label: "高清", note: "最高 320k" },
];
/** 免费上限由各平台自己决定:QQ/酷我匿名只给 128k,升档不会变(不试探付费文件) */
const qualitySupported = computed(() => source.value === "netease" || source.value === "kugou");
const qualityNote = computed(() => {
  if (!qualitySupported.value) return "该音源免费上限 128k";
  return onlineQuality.value === "high" ? "320k · 平台免费上限" : "128k · 更省流量";
});

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

/** 正在保存的曲目 id:一次只跑一首,并发写库会让扫描与指纹互相打架 */
const savingId = ref("");

/** 保存到本地库:与播放同一道闸,付费曲目取链就失败,不存在绕过 */
async function save(t: OnlineTrack): Promise<void> {
  if (!playable(t) || savingId.value) return;
  savingId.value = t.id;
  try {
    const r = await api.onlineDownload(
      t.source,
      t.id,
      { name: t.name, artist: t.artist, album: t.album, cover: t.cover },
      onlineQuality.value,
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
    savingId.value = "";
  }
}
</script>

<template>
  <div class="view">
    <div class="online-shell">
      <header class="ov-head">
        <h1>在线音乐</h1>
        <p class="sub">搜索并试听各平台免费曲目 · 可保存到本地库 · 支持正版</p>
      </header>

      <section class="search-panel">
        <div class="panel-top">
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
          <span class="panel-hint">仅免费曲目 · 无需登录</span>
        </div>

        <div class="panel-opts">
          <span class="opt-label">音质</span>
          <div class="quals" role="radiogroup" aria-label="在线音质">
            <button
              v-for="q in QUALITIES"
              :key="q.id"
              type="button"
              role="radio"
              :aria-checked="onlineQuality === q.id"
              class="qual"
              :class="{ on: onlineQuality === q.id }"
              :disabled="!qualitySupported"
              :title="
                qualitySupported ? `${q.label} ${q.note}` : '该音源免费上限 128k,无需选择'
              "
              @click="setOnlineQuality(q.id)"
            >
              {{ q.label }}
              <span class="qual-note">{{ q.note }}</span>
            </button>
          </div>
          <span class="opt-hint">{{ qualityNote }}</span>
        </div>

        <form class="ov-search" @submit.prevent="search">
          <svg class="s-ico" viewBox="0 0 24 24" aria-hidden="true">
            <circle cx="11" cy="11" r="6.5" fill="none" stroke="currentColor" stroke-width="1.8" />
            <path d="m16 16 4.5 4.5" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
          </svg>
          <input
            v-model="keyword"
            type="search"
            enterkeyhint="search"
            aria-label="搜索在线音乐"
            placeholder="搜索歌曲 / 艺人 / 专辑"
            @keydown.enter.prevent="search"
          />
          <button v-if="keyword" class="clear" type="button" title="清空" @click="keyword = ''">
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d="m7 7 10 10M17 7 7 17" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
            </svg>
          </button>
          <button class="go" type="submit" :disabled="!keyword.trim() || phase === 'loading'">
            {{ phase === "loading" && results.length === 0 ? "搜索中…" : "搜索" }}
          </button>
        </form>
      </section>

      <div v-if="phase === 'err'" class="state-card">
        <span class="state-icon state-icon-error">
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <circle cx="12" cy="12" r="8.2" fill="none" stroke="currentColor" stroke-width="1.7" />
            <path d="M12 7.8v5.4" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
            <circle cx="12" cy="16.4" r="0.9" fill="currentColor" />
          </svg>
        </span>
        <h2>搜索失败</h2>
        <p>{{ errMsg }}</p>
        <button class="state-action" type="button" @click="search">重试</button>
      </div>

      <div v-else-if="phase === 'loading' && results.length === 0" class="state-card">
        <span class="state-spinner" aria-hidden="true"></span>
        <h2>正在搜索…</h2>
        <p>正在连接 {{ sourceLabel }}</p>
      </div>

      <div v-else-if="phase === 'done' && results.length === 0" class="state-card">
        <span class="state-icon">
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <circle cx="11" cy="11" r="6.5" fill="none" stroke="currentColor" stroke-width="1.7" />
            <path d="m16 16 4.5 4.5" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" />
            <path d="M8.6 11h4.8" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" />
          </svg>
        </span>
        <h2>没有找到“{{ keyword }}”</h2>
        <p>换个关键词,或切换其它音源试试</p>
      </div>

      <section v-else-if="results.length" class="results">
        <div class="results-head">
          <h2>搜索结果</h2>
          <span>已加载 {{ results.length }} 首 · {{ sourceLabel }}</span>
        </div>

        <div class="ov-list">
          <div
            v-for="(t, i) in results"
            :key="t.id"
            class="ov-row"
            :class="{ dim: !playable(t), saving: savingId === t.id }"
          >
            <button
              class="ov-main"
              :title="playable(t) ? `播放 ${t.name}` : `${t.name} 需要会员或暂无版权`"
              @click="play(t)"
            >
              <span class="no">
                <span class="num">{{ i + 1 }}</span>
                <svg v-if="playable(t)" class="row-play" viewBox="0 0 24 24" aria-hidden="true">
                  <path d="M8.2 5.5v13L19 12z" fill="currentColor" />
                </svg>
              </span>
              <span class="meta">
                <span class="t">{{ t.name }}</span>
                <span class="a">{{ t.artist }}<template v-if="t.album"> · {{ t.album }}</template></span>
              </span>
              <span v-if="!playable(t)" class="vip">VIP</span>
              <span class="dur">{{ fmtTime(t.duration) }}</span>
            </button>
            <button
              v-if="playable(t)"
              class="row-save"
              type="button"
              :disabled="savingId !== ''"
              :aria-label="`保存 ${t.name} 到本地库`"
              :title="savingId === t.id ? '正在下载…' : '保存到本地库'"
              @click="save(t)"
            >
              <span v-if="savingId === t.id" class="row-spin" aria-hidden="true"></span>
              <svg v-else viewBox="0 0 24 24" aria-hidden="true">
                <path d="M12 4.6v9.2m0 0 3.4-3.4M12 13.8l-3.4-3.4" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
                <path d="M5.4 18.4h13.2" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
              </svg>
            </button>
          </div>
        </div>

        <button v-if="lastPage >= 0" class="more" :disabled="phase === 'loading'" @click="loadMore">
          {{ phase === "loading" ? "加载中…" : "加载更多" }}
        </button>
        <p v-else-if="results.length >= 30" class="end-hint">没有更多了</p>
      </section>

      <div v-else class="state-card state-idle">
        <span class="state-icon">
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M9.2 17.3V7l9-1.9v10.2" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round" />
            <circle cx="6.9" cy="17.4" r="2.3" fill="none" stroke="currentColor" stroke-width="1.7" />
            <circle cx="15.9" cy="15.4" r="2.3" fill="none" stroke="currentColor" stroke-width="1.7" />
          </svg>
        </span>
        <h2>搜索在线音乐</h2>
        <p>支持网易云、QQ 音乐、酷狗音乐与酷我音乐</p>
        <span class="state-hint">输入关键词,按回车或点击搜索</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.online-shell {
  width: min(100%, 1080px);
  margin: 0 auto;
}
.ov-head {
  margin-bottom: 18px;
}
.ov-head h1 {
  font-size: 30px;
  font-weight: 700;
  letter-spacing: 0.2px;
}
.sub {
  margin-top: 4px;
  font-size: 13px;
  color: var(--text-2);
}
.search-panel {
  padding: 14px;
  border-radius: 18px;
  border: 1px solid var(--hairline);
  background: color-mix(in srgb, var(--bg-2) 68%, transparent);
  box-shadow: 0 18px 48px rgba(0, 0, 0, 0.1);
  backdrop-filter: blur(16px);
  -webkit-backdrop-filter: blur(16px);
}
.panel-top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}
.panel-hint {
  flex: none;
  font-size: 11.5px;
  color: var(--text-3);
  white-space: nowrap;
}
.panel-opts {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 10px;
}
.opt-label {
  flex: none;
  font-size: 11.5px;
  font-weight: 600;
  color: var(--text-3);
}
.opt-hint {
  font-size: 11.5px;
  color: var(--text-3);
}
.quals {
  display: inline-flex;
  gap: 2px;
  padding: 3px;
  border-radius: 999px;
  background: var(--pill-bg);
  border: 1px solid var(--hairline);
}
.qual {
  display: inline-flex;
  align-items: baseline;
  gap: 5px;
  height: 26px;
  padding: 0 12px;
  border: 0;
  border-radius: 999px;
  background: transparent;
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text-2);
  transition:
    background 0.18s ease,
    color 0.18s ease;
}
.qual:hover:not(:disabled) {
  color: var(--text);
  background: var(--hover);
}
.qual.on {
  background: var(--accent);
  color: #fff;
}
.qual:disabled {
  opacity: 0.45;
  cursor: default;
}
.qual:focus-visible {
  outline: 2px solid color-mix(in srgb, var(--accent) 45%, transparent);
  outline-offset: 2px;
}
.qual-note {
  font-size: 10.5px;
  font-weight: 500;
  opacity: 0.7;
}
.sources {
  display: inline-flex;
  gap: 2px;
  padding: 3px;
  border-radius: 999px;
  background: var(--pill-bg);
  border: 1px solid var(--hairline);
}
.src {
  height: 30px;
  padding: 0 16px;
  border: 0;
  border-radius: 999px;
  background: transparent;
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text-2);
  transition:
    background 0.18s ease,
    color 0.18s ease,
    box-shadow 0.18s ease;
}
.src:hover {
  color: var(--text);
  background: var(--hover);
}
.src.on {
  background: var(--accent);
  color: #fff;
  box-shadow: 0 4px 12px color-mix(in srgb, var(--accent) 30%, transparent);
}
.src:focus-visible {
  outline: 2px solid color-mix(in srgb, var(--accent) 45%, transparent);
  outline-offset: 2px;
}
.ov-search {
  margin-top: 12px;
  display: flex;
  align-items: center;
  gap: 10px;
  height: 48px;
  padding: 0 6px 0 14px;
  border-radius: 14px;
  border: 1px solid var(--hairline);
  background: color-mix(in srgb, var(--bg) 72%, transparent);
  transition:
    border-color 0.2s ease,
    box-shadow 0.2s ease,
    background 0.2s ease;
}
.ov-search:focus-within {
  border-color: color-mix(in srgb, var(--accent) 55%, var(--hairline));
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 16%, transparent);
  background: color-mix(in srgb, var(--bg) 88%, transparent);
}
.s-ico {
  flex: none;
  width: 18px;
  height: 18px;
  color: var(--text-3);
}
.ov-search input {
  flex: 1;
  min-width: 0;
  height: 100%;
  border: 0;
  outline: none;
  background: transparent;
  color: var(--text);
  font-size: 14.5px;
}
.ov-search input::placeholder {
  color: var(--text-3);
}
.clear {
  flex: none;
  width: 28px;
  height: 28px;
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
  height: 36px;
  padding: 0 20px;
  border: 0;
  border-radius: 999px;
  background: var(--accent);
  color: #fff;
  font-size: 13.5px;
  font-weight: 600;
  transition: background 0.18s ease;
}
.go:disabled {
  opacity: 0.45;
  cursor: default;
}
.go:hover:not(:disabled) {
  background: var(--accent-hover);
}

.state-card {
  min-height: 300px;
  margin-top: 20px;
  padding: 32px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 9px;
  text-align: center;
  border-radius: 18px;
  border: 1px solid var(--hairline);
  background: color-mix(in srgb, var(--bg-2) 62%, transparent);
  backdrop-filter: blur(14px);
  -webkit-backdrop-filter: blur(14px);
}
.state-card h2 {
  font-size: 19px;
  font-weight: 700;
  color: var(--text);
}
.state-card p {
  font-size: 13px;
  color: var(--text-2);
}
.state-hint {
  margin-top: 2px;
  font-size: 12px;
  color: var(--text-3);
}
.state-icon {
  width: 52px;
  height: 52px;
  margin-bottom: 6px;
  border-radius: 50%;
  display: grid;
  place-items: center;
  color: var(--accent);
  background: color-mix(in srgb, var(--accent) 12%, transparent);
  border: 1px solid color-mix(in srgb, var(--accent) 20%, transparent);
}
.state-icon svg {
  width: 24px;
  height: 24px;
}
.state-icon-error {
  color: #e0a23a;
  background: color-mix(in srgb, #e0a23a 12%, transparent);
  border-color: color-mix(in srgb, #e0a23a 22%, transparent);
}
.state-spinner {
  width: 34px;
  height: 34px;
  margin-bottom: 8px;
  border-radius: 50%;
  border: 3px solid var(--hairline);
  border-top-color: var(--accent);
  animation: ov-spin 0.8s linear infinite;
}
.state-action {
  margin-top: 8px;
  height: 32px;
  padding: 0 20px;
  border-radius: 999px;
  background: var(--accent);
  color: #fff;
  font-size: 12.5px;
  font-weight: 600;
}
.state-action:hover {
  background: var(--accent-hover);
}
@keyframes ov-spin {
  to {
    transform: rotate(360deg);
  }
}

.results-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
  margin: 22px 2px 8px;
}
.results-head h2 {
  font-size: 15px;
  font-weight: 700;
}
.results-head span {
  font-size: 11.5px;
  color: var(--text-3);
}
.ov-list {
  display: flex;
  flex-direction: column;
}
.ov-row {
  display: flex;
  align-items: center;
  border-radius: 9px;
  transition: background 0.2s var(--ease-out-soft);
}
.ov-row + .ov-row {
  box-shadow: inset 0 1px 0 color-mix(in srgb, var(--hairline) 55%, transparent);
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
  padding: 10px 12px;
  text-align: left;
}
.ov-main:active {
  transform: scale(0.995);
}
.no {
  position: relative;
  width: 22px;
  flex: none;
  text-align: center;
  font-size: 12px;
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
}
.row-play {
  display: none;
  width: 14px;
  height: 14px;
  margin: 0 auto;
  color: var(--accent);
}
.ov-row:not(.dim):hover .num {
  display: none;
}
.ov-row:not(.dim):hover .row-play {
  display: block;
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
/* 保存按钮:行 hover 才现身,和行内播放图标一个脾气;正在保存的那行常驻并转圈 */
.row-save {
  flex: none;
  width: 32px;
  height: 32px;
  margin-right: 8px;
  border-radius: 50%;
  display: none;
  align-items: center;
  justify-content: center;
  color: var(--text-2);
}
.ov-row:hover .row-save,
.row-save:focus-visible,
.ov-row.saving .row-save {
  display: flex;
}
.row-save:hover:not(:disabled) {
  background: var(--hover);
  color: var(--accent);
}
.row-save:disabled {
  opacity: 0.6;
  cursor: default;
}
.row-save svg {
  width: 15px;
  height: 15px;
}
.row-spin {
  width: 14px;
  height: 14px;
  border-radius: 50%;
  border: 2px solid var(--hairline);
  border-top-color: var(--accent);
  animation: ov-spin 0.7s linear infinite;
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

@media (max-width: 760px) {
  .panel-hint,
  .opt-hint {
    display: none;
  }
  .search-panel {
    padding: 10px;
  }
  .src {
    padding: 0 12px;
  }
  .qual {
    padding: 0 10px;
  }
  .qual-note {
    display: none;
  }
}
</style>
