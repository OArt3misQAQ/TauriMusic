<script setup lang="ts">
import { computed, onMounted } from "vue";
import { lib, zhCollator, trackAlbumKey } from "../stores/library";
import { loadStats, stats, statsLoadError, todayStr } from "../stores/stats";

/** 听歌统计页:全部聚合都从「单曲统计 × 当前曲库元数据」即时推导,
 *  统计文件里不冗余存歌手/专辑——标签修改后各榜单自动跟随曲库走 */

onMounted(() => void loadStats());

/** 拉取失败可重试,不能永久停在"正在加载" */
function retry(): void {
  statsLoadError.value = null;
  void loadStats();
}

const trackMeta = computed(() => {
  const m = new Map<string, { title: string; artist: string; album: string }>();
  for (const t of lib.tracks) {
    m.set(t.id, { title: t.title, artist: t.artist, album: t.album });
  }
  return m;
});

/** 累计秒数/次数/在库歌曲数(已删除曲目的时长计入总量,不再进榜单) */
const totals = computed(() => {
  let sec = 0;
  let count = 0;
  let songs = 0;
  const s = stats.value;
  if (s) {
    for (const [id, t] of Object.entries(s.tracks)) {
      sec += t.sec;
      count += t.count;
      if (t.sec > 0 && trackMeta.value.has(id)) songs += 1;
    }
  }
  return { sec, count, songs };
});

function dayKeyOffset(daysAgo: number): string {
  const d = new Date();
  d.setDate(d.getDate() - daysAgo);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

const todaySec = computed(() => stats.value?.days[todayStr()] ?? 0);
const weekSec = computed(() => {
  let sum = 0;
  for (let i = 0; i < 7; i++) sum += stats.value?.days[dayKeyOffset(i)] ?? 0;
  return sum;
});

/** 最近 30 天(含今天)的日时长序列,缺数据的天补 0 */
const last30 = computed(() => {
  const arr: { key: string; sec: number }[] = [];
  for (let i = 29; i >= 0; i--) {
    const key = dayKeyOffset(i);
    arr.push({ key, sec: stats.value?.days[key] ?? 0 });
  }
  return arr;
});
const dayMax = computed(() => Math.max(...last30.value.map((d) => d.sec), 1));

/** Top 歌曲:按累计时长排,已删除曲目不出现在榜单 */
const topTracks = computed(() => {
  const s = stats.value;
  if (!s) return [];
  return Object.entries(s.tracks)
    .map(([id, st]) => ({ id, ...st, meta: trackMeta.value.get(id) }))
    .filter((t) => t.meta && t.sec > 0)
    .sort((a, b) => b.sec - a.sec)
    .slice(0, 20);
});

/** 最近播放:按最后收听时刻排 */
const recentTracks = computed(() => {
  const s = stats.value;
  if (!s) return [];
  return Object.entries(s.tracks)
    .map(([id, st]) => ({ id, ...st, meta: trackMeta.value.get(id) }))
    .filter((t) => t.meta && t.last > 0)
    .sort((a, b) => b.last - a.last)
    .slice(0, 20);
});

/** 艺人榜:多艺人(/ & 、分隔)各记各的 */
const topArtists = computed(() => {
  const s = stats.value;
  if (!s) return [];
  const agg = new Map<string, number>();
  for (const [id, st] of Object.entries(s.tracks)) {
    const meta = trackMeta.value.get(id);
    if (!meta) continue;
    for (const seg of meta.artist.split(/[/&、]/)) {
      const name = seg.trim();
      if (!name) continue;
      agg.set(name, (agg.get(name) ?? 0) + st.sec);
    }
  }
  return [...agg.entries()]
    .map(([name, sec]) => ({ name, sec }))
    .sort((a, b) => b.sec - a.sec || zhCollator.compare(a.name, b.name))
    .slice(0, 10);
});

/** 专辑榜:按曲目所属专辑键聚合(id→专辑键 先建映射,避免逐条线性查找) */
const topAlbums = computed(() => {
  const s = stats.value;
  if (!s) return [];
  const keyById = new Map<string, string>();
  const nameOf = new Map<string, { album: string; artist: string }>();
  for (const t of lib.tracks) {
    keyById.set(t.id, trackAlbumKey(t));
    nameOf.set(trackAlbumKey(t), { album: t.album, artist: t.albumArtist || t.artist });
  }
  const agg = new Map<string, number>();
  for (const [id, st] of Object.entries(s.tracks)) {
    const key = keyById.get(id);
    if (!key) continue;
    agg.set(key, (agg.get(key) ?? 0) + st.sec);
  }
  return [...agg.entries()]
    .map(([key, sec]) => ({ key, sec, ...(nameOf.get(key) ?? { album: key, artist: "" }) }))
    .sort((a, b) => b.sec - a.sec)
    .slice(0, 10);
});

const hasData = computed(() => totals.value.sec > 0);
const barMax = computed(() => Math.max(...topArtists.value.map((a) => a.sec), ...topAlbums.value.map((a) => a.sec), 1));

/** 时长中文格式:秒 → 「x 小时 y 分钟」/「y 分钟」/「y 分 z 秒」 */
function fmtDur(sec: number): string {
  if (!isFinite(sec) || sec <= 0) return "0 秒";
  const total = Math.round(sec);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (h > 0) return m > 0 ? `${h} 小时 ${m} 分钟` : `${h} 小时`;
  if (m > 0) return `${m} 分钟`;
  return `${s} 秒`;
}
function fmtCount(n: number): string {
  return n >= 10000 ? `${(n / 10000).toFixed(1)} 万` : String(n);
}
function fmtDate(unix: number): string {
  if (!unix) return "—";
  const d = new Date(unix * 1000);
  const today = todayStr();
  const that = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
  if (that === today) return `今天 ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  return that.slice(5); // MM-DD
}
function chartLabel(key: string): string {
  const [, m, d] = key.split("-");
  return `${Number(m)}/${Number(d)}`;
}
</script>

<template>
  <div class="view">
    <header class="view-head">
      <h1>听歌统计</h1>
      <div class="right">
        <span class="privacy">数据仅保存在本机 · 绝不联网</span>
      </div>
    </header>

    <div v-if="statsLoadError" class="empty">
      <h2>统计数据读取失败</h2>
      <button class="mini ghost" @click="retry">重试</button>
    </div>
    <div v-else-if="!stats" class="empty"><h2>正在加载统计数据…</h2></div>

    <div v-else-if="!hasData" class="empty">
      <svg class="icon" width="64" height="64" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round">
        <path d="M5 20V10M12 20V4M19 20v-7" />
      </svg>
      <h2>还没有统计数据</h2>
      <p>开始播放音乐后,这里会逐渐积累你的收听足迹</p>
    </div>

    <div v-else class="body">
      <!-- 总览卡 -->
      <div class="cards">
        <div class="card">
          <span class="k">累计收听</span>
          <span class="v">{{ fmtDur(totals.sec) }}</span>
          <span class="s">本周 {{ fmtDur(weekSec) }}</span>
        </div>
        <div class="card">
          <span class="k">播放次数</span>
          <span class="v">{{ fmtCount(totals.count) }}</span>
          <span class="s">听过 {{ totals.songs }} 首</span>
        </div>
        <div class="card">
          <span class="k">今日收听</span>
          <span class="v">{{ fmtDur(todaySec) }}</span>
          <span class="s">截至 {{ new Date().getHours() }} 点</span>
        </div>
        <div class="card chart-card">
          <span class="k">最近 30 天</span>
          <div class="chart">
            <div v-for="d in last30" :key="d.key" class="col" :title="`${d.key} · ${fmtDur(d.sec)}`">
              <div class="bar" :style="{ height: `${Math.max((d.sec / dayMax) * 100, d.sec > 0 ? 4 : 0)}%` }"></div>
            </div>
          </div>
          <span class="s">{{ chartLabel(last30[0].key) }} — {{ chartLabel(last30[29].key) }}</span>
        </div>
      </div>

      <!-- 歌手 / 专辑双榜 -->
      <div class="two-col">
        <section class="panel">
          <h2>最常听的歌手</h2>
          <ol class="bars">
            <li v-for="(a, i) in topArtists" :key="a.name">
              <span class="idx">{{ i + 1 }}</span>
              <div class="bar-wrap">
                <div class="bar-line">
                  <span class="name" :title="a.name">{{ a.name }}</span>
                  <span class="sec">{{ fmtDur(a.sec) }}</span>
                </div>
                <div class="track"><div class="fill" :style="{ width: `${(a.sec / barMax) * 100}%` }"></div></div>
              </div>
            </li>
          </ol>
        </section>
        <section class="panel">
          <h2>最常听的专辑</h2>
          <ol class="bars">
            <li v-for="(a, i) in topAlbums" :key="a.key">
              <span class="idx">{{ i + 1 }}</span>
              <div class="bar-wrap">
                <div class="bar-line">
                  <span class="name" :title="`${a.artist} - ${a.album}`">{{ a.album }}</span>
                  <span class="sec">{{ fmtDur(a.sec) }}</span>
                </div>
                <div class="track"><div class="fill" :style="{ width: `${(a.sec / barMax) * 100}%` }"></div></div>
              </div>
            </li>
          </ol>
        </section>
      </div>

      <!-- 歌曲榜 / 最近播放 -->
      <div class="two-col">
        <section class="panel">
          <h2>听歌排行</h2>
          <ol class="rows">
            <li v-for="t in topTracks" :key="t.id">
              <span class="idx">{{ fmtCount(t.count) }} 次</span>
              <div class="row-main">
                <span class="name" :title="t.meta!.title">{{ t.meta!.title }}</span>
                <span class="sub">{{ t.meta!.artist }} · {{ fmtDur(t.sec) }}</span>
              </div>
            </li>
          </ol>
        </section>
        <section class="panel">
          <h2>最近在听</h2>
          <ol class="rows">
            <li v-for="t in recentTracks" :key="t.id">
              <span class="idx">{{ fmtDate(t.last) }}</span>
              <div class="row-main">
                <span class="name" :title="t.meta!.title">{{ t.meta!.title }}</span>
                <span class="sub">{{ t.meta!.artist }} · {{ t.meta!.album }}</span>
              </div>
            </li>
          </ol>
        </section>
      </div>
    </div>
  </div>
</template>

<style scoped>
.view {
  height: 100%;
  overflow-y: auto;
  padding: 22px 28px 40px;
}
.privacy {
  font-size: 12px;
  color: var(--text-3);
}
.empty {
  height: 70%;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  color: var(--text-2);
}
.empty .icon {
  color: var(--text-3);
  margin-bottom: 6px;
}
.empty h2 {
  font-size: 17px;
  color: var(--text);
}
.empty p {
  font-size: 13.5px;
}

.body {
  display: flex;
  flex-direction: column;
  gap: 18px;
  margin-top: 16px;
}

.cards {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 12px;
}
.card {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 14px 16px;
  border-radius: 12px;
  background: var(--bg-2);
  border: 1px solid var(--hairline);
}
.card .k {
  font-size: 12px;
  color: var(--text-3);
}
.card .v {
  font-size: 20px;
  font-weight: 700;
  color: var(--text);
  font-variant-numeric: tabular-nums;
}
.card .s {
  font-size: 11.5px;
  color: var(--text-3);
}
.chart-card .chart {
  display: flex;
  align-items: flex-end;
  gap: 2px;
  height: 44px;
  margin: 2px 0;
}
.chart-card .col {
  flex: 1;
  height: 100%;
  display: flex;
  align-items: flex-end;
}
.chart-card .bar {
  width: 100%;
  border-radius: 2px 2px 0 0;
  background: var(--accent);
  opacity: 0.85;
  min-height: 0;
}

.two-col {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 12px;
}
.panel {
  padding: 16px 18px;
  border-radius: 12px;
  background: var(--bg-2);
  border: 1px solid var(--hairline);
  min-width: 0;
}
.panel h2 {
  font-size: 14.5px;
  font-weight: 700;
  color: var(--text);
  margin-bottom: 12px;
}

.bars {
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.bars li {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
}
.idx {
  width: 34px;
  flex: none;
  font-size: 11.5px;
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
  text-align: right;
}
.bar-wrap {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.bar-line {
  display: flex;
  justify-content: space-between;
  gap: 10px;
}
.bar-line .name {
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.bar-line .sec {
  font-size: 11.5px;
  color: var(--text-3);
  flex: none;
  font-variant-numeric: tabular-nums;
}
.track {
  height: 5px;
  border-radius: 999px;
  background: var(--pill-bg);
  overflow: hidden;
}
.fill {
  height: 100%;
  border-radius: inherit;
  background: var(--accent);
  transition: width 0.4s var(--ease-out-soft);
}

.rows {
  display: flex;
  flex-direction: column;
}
.rows li {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 7px 0;
  border-bottom: 1px solid var(--hairline);
}
.rows li:last-child {
  border-bottom: none;
}
.row-main {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}
.row-main .name {
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.row-main .sub {
  font-size: 11.5px;
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

@media (max-width: 900px) {
  .cards {
    grid-template-columns: repeat(2, 1fr);
  }
  .two-col {
    grid-template-columns: 1fr;
  }
}
</style>
