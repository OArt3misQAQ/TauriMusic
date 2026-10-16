<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { RecycleScroller } from "vue-virtual-scroller";
import { convertFileSrc } from "@tauri-apps/api/core";
import type { Track } from "../api";
import { current, fmtTime, playTracks, playTrack } from "../stores/player";
import { exportTrackTmc, exportTracksTmc, deleteTracks, trackAlbumKey } from "../stores/library";
import { useConfirmableAction } from "../composables/useConfirmableAction";
import { openPlaylistPicker } from "../stores/playlists";
import { openAlbum, openArtist } from "../stores/ui";
import { openCtx } from "../stores/context";

const props = defineProps<{
  tracks: Track[];
  /** 显示序号(专辑内曲目) */
  showIndex?: boolean;
  /** 显示封面缩略图 */
  showCover?: boolean;
  /** 显示专辑列 */
  showAlbum?: boolean;
  /** 虚拟滚动:大列表(整库/专辑)开启,父级需提供有界高度 */
  virtual?: boolean;
}>();

/** 虚拟模式固定行高:有封面 52px,无封面 47px(行内行高在 .virtual 中固定) */
const rowHeight = computed(() => (props.showCover ? 52 : 47));

function play(t: Track): void {
  playTrack(t, props.tracks);
}

function exportOne(t: Track): void {
  void exportTrackTmc(t.id, t.title, t.artist);
}

/** 删除采用两段确认:第一次点进入确认态,3 秒内再点执行 */
const { confirmingId, confirm } = useConfirmableAction();

function onDelete(t: Track): void {
  if (confirm(t.id)) void deleteTracks([t.id]);
}

/** 多选:Ctrl/Shift 点击进入选择;Shift 为范围选 */
const selected = ref<Set<string>>(new Set());
let anchorIndex = -1;

// 曲目列表被替换(搜索词收窄/删除后)时修剪选择集:残留的不可见 id 仍会随
// 批量删除一起提交,操作条计数也会虚高;Shift 范围选的锚点随之失效,必须重置
watch(
  () => props.tracks,
  () => {
    if (selected.value.size > 0) {
      const visible = new Set(props.tracks.map((t) => t.id));
      selected.value = new Set([...selected.value].filter((id) => visible.has(id)));
    }
    anchorIndex = -1;
  },
);

function toggleSelect(t: Track, i: number): void {
  const next = new Set(selected.value);
  if (next.has(t.id)) next.delete(t.id);
  else next.add(t.id);
  selected.value = next;
  anchorIndex = i;
}

function rangeSelect(to: number): void {
  const from = anchorIndex >= 0 ? anchorIndex : 0;
  const [a, b] = from <= to ? [from, to] : [to, from];
  const next = new Set<string>();
  for (let i = a; i <= b; i++) next.add(props.tracks[i]?.id ?? "");
  next.delete("");
  selected.value = next;
}

function rowClick(e: MouseEvent, t: Track, i: number): void {
  if (e.shiftKey) {
    rangeSelect(i);
    return;
  }
  if (e.ctrlKey || e.metaKey) {
    toggleSelect(t, i);
    return;
  }
  if (selected.value.size > 0) {
    // 已处于选择模式时,普通点击重置为只选这一行
    selected.value = new Set([t.id]);
    anchorIndex = i;
    return;
  }
  play(t);
}

function clearSelection(): void {
  selected.value = new Set();
  anchorIndex = -1;
}

const selectedTracks = () => props.tracks.filter((t) => selected.value.has(t.id));

function playSelected(): void {
  const list = selectedTracks();
  if (list.length) playTracks(list, 0);
  clearSelection();
}

function exportSelected(): void {
  void exportTracksTmc([...selected.value], props.tracks);
  clearSelection();
}

const { confirmingId: confirmingBatchDelete, confirm: confirmBatch } = useConfirmableAction();

function deleteSelected(): void {
  // 按当前可见曲目过滤后提交,与"播放/导出选中"的行为口径一致
  if (confirmBatch("__batch__")) {
    void deleteTracks(selectedTracks().map((t) => t.id)).then(clearSelection);
  }
}

/** 歌曲右键菜单:有选中时批量操作,否则单首 */
function rowMenu(e: MouseEvent, t: Track): void {
  if (selected.value.has(t.id) && selected.value.size > 1) {
    const n = selected.value.size;
    openCtx(e, [
      { label: `播放选中的 ${n} 首`, icon: "play", action: playSelected },
      { label: `加入歌单(${n} 首)`, icon: "playlist", action: () => openPlaylistPicker([...selected.value]) },
      { label: `导出选中为 TMC 音乐包(${n})`, icon: "export", action: exportSelected },
      {
        label: `删除选中的 ${n} 首(移入回收站)`,
        icon: "delete",
        danger: true,
        action: deleteSelected,
      },
    ]);
    return;
  }
  openCtx(e, [
    { label: "播放", icon: "play", action: () => play(t) },
    { label: "查看专辑", icon: "album", action: () => openAlbum(trackAlbumKey(t)) },
    { label: "查看艺人", icon: "artist", action: () => openArtist(t.artist) },
    { label: "添加到播放列表…", icon: "playlist", action: () => openPlaylistPicker([t.id]) },
    { label: "导出为 TMC 音乐包", icon: "export", action: () => exportOne(t) },
    { label: "删除(移入回收站)", icon: "delete", danger: true, action: () => void deleteTracks([t.id]) },
  ]);
}
</script>

<template>
  <!-- 虚拟模式:仅渲染视口内的行(万首歌曲 DOM 从数万节点降到 ~20 行) -->
  <RecycleScroller
    v-if="virtual"
    class="tl virtual"
    :items="tracks"
    :item-size="rowHeight"
    key-field="id"
    :buffer="300"
  >
    <template #default="{ item: t, index: i, active }">
      <div
        class="row"
        :style="{ height: rowHeight + 'px' }"
        :class="{ current: t.id === current?.id, picked: selected.has(t.id), hovered: active }"
        @click="rowClick($event, t, i)"
        @contextmenu="rowMenu($event, t)"
      >
        <span v-if="showIndex" class="idx">
          <span class="num">{{ t.trackNo ?? i + 1 }}</span>
          <svg class="play-ico" viewBox="0 0 24 24"><path d="M8.6 6.5v11L18 12z" fill="currentColor" /></svg>
        </span>
        <span v-if="showCover" class="cov">
          <img v-if="t.cover" :src="convertFileSrc(t.cover)" loading="lazy" decoding="async" alt="" />
          <span v-else class="ph">♪</span>
          <svg class="play-ico" viewBox="0 0 24 24"><path d="M8.6 6.5v11L18 12z" fill="currentColor" /></svg>
        </span>
        <span class="main">
          <span class="t" :title="t.title">{{ t.title }}</span>
          <span class="a" :title="t.artist">{{ t.artist }}</span>
        </span>
        <span v-if="showAlbum" class="al" :title="t.album">{{ t.album }}</span>
        <span v-if="t.hasLyrics" class="badge">词</span>
        <button class="exp" title="导出为 TMC 音乐包" @click.stop="exportOne(t)">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 13.5V4M8.5 7.5 12 4l3.5 3.5" />
            <path d="M5 15v2.6c0 .77.63 1.4 1.4 1.4h11.2c.77 0 1.4-.63 1.4-1.4V15" />
          </svg>
        </button>
        <button
          class="del"
          :class="{ confirm: confirmingId === t.id }"
          :title="confirmingId === t.id ? '再次点击确认删除(移入回收站)' : '删除这首歌(移入回收站)'"
          @click.stop="onDelete(t)"
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <path d="M4.5 7h15M9.5 7V5.2c0-.66.54-1.2 1.2-1.2h2.6c.66 0 1.2.54 1.2 1.2V7M6.5 7l.8 11.3c.06.77.7 1.2 1.4 1.2h6.6c.7 0 1.34-.43 1.4-1.2L17.5 7" />
            <path d="M10 11v5M14 11v5" />
          </svg>
        </button>
        <span class="dur">{{ fmtTime(t.duration) }}</span>
      </div>
    </template>
  </RecycleScroller>

  <div v-else class="tl">
    <div
      v-for="(t, i) in tracks"
      :key="t.id"
      class="row"
      :class="{ current: t.id === current?.id, picked: selected.has(t.id) }"
      @click="rowClick($event, t, i)"
      @contextmenu="rowMenu($event, t)"
    >
      <span v-if="showIndex" class="idx">
        <span class="num">{{ t.trackNo ?? i + 1 }}</span>
        <svg class="play-ico" viewBox="0 0 24 24"><path d="M8.6 6.5v11L18 12z" fill="currentColor" /></svg>
      </span>
      <span v-if="showCover" class="cov">
        <img v-if="t.cover" :src="convertFileSrc(t.cover)" loading="lazy" decoding="async" alt="" />
        <span v-else class="ph">♪</span>
        <svg class="play-ico" viewBox="0 0 24 24"><path d="M8.6 6.5v11L18 12z" fill="currentColor" /></svg>
      </span>
      <span class="main">
        <span class="t" :title="t.title">{{ t.title }}</span>
        <span class="a" :title="t.artist">{{ t.artist }}</span>
      </span>
      <span v-if="showAlbum" class="al" :title="t.album">{{ t.album }}</span>
      <span v-if="t.hasLyrics" class="badge">词</span>
      <button class="exp" title="导出为 TMC 音乐包" @click.stop="exportOne(t)">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
          <path d="M12 13.5V4M8.5 7.5 12 4l3.5 3.5" />
          <path d="M5 15v2.6c0 .77.63 1.4 1.4 1.4h11.2c.77 0 1.4-.63 1.4-1.4V15" />
        </svg>
      </button>
      <button
        class="del"
        :class="{ confirm: confirmingId === t.id }"
        :title="confirmingId === t.id ? '再次点击确认删除(移入回收站)' : '删除这首歌(移入回收站)'"
        @click.stop="onDelete(t)"
      >
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
          <path d="M4.5 7h15M9.5 7V5.2c0-.66.54-1.2 1.2-1.2h2.6c.66 0 1.2.54 1.2 1.2V7M6.5 7l.8 11.3c.06.77.7 1.2 1.4 1.2h6.6c.7 0 1.34-.43 1.4-1.2L17.5 7" />
          <path d="M10 11v5M14 11v5" />
        </svg>
      </button>
      <span class="dur">{{ fmtTime(t.duration) }}</span>
    </div>
  </div>

  <!-- 多选操作条(虚拟模式下为固定浮层) -->
  <Transition name="bar">
    <div v-if="selected.size > 0" class="sel-bar" :class="{ fixed: virtual }">
        <span class="sel-count">已选 {{ selected.size }} 首</span>
        <div class="sel-actions">
          <button class="sel-btn primary" @click="playSelected">播放</button>
          <button class="sel-btn" @click="openPlaylistPicker([...selected])">加入歌单</button>
          <button class="sel-btn" @click="exportSelected">导出 TMC</button>
          <button class="sel-btn danger" :class="{ confirm: confirmingBatchDelete }" @click="deleteSelected">
            {{ confirmingBatchDelete ? "确认删除?" : "删除" }}
          </button>
          <button class="sel-btn ghost" @click="clearSelection">取消</button>
        </div>
      </div>
  </Transition>
</template>

<style scoped>
.tl {
  display: flex;
  flex-direction: column;
}

.row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 6px 10px;
  border-radius: 8px;
  cursor: pointer;
  min-width: 0;
  transition: background 0.25s var(--ease-out-soft);
  /* 大曲库性能:视口外的行跳过布局与绘制(非虚拟模式生效;虚拟模式自带回收) */
  content-visibility: auto;
  contain-intrinsic-size: auto 54px;
}
/* 虚拟模式:固定行内行高,保证 item-size 与实际渲染一致 */
.virtual .t {
  line-height: 18px;
}
.virtual .a {
  line-height: 16px;
}
.row:hover {
  background: var(--hover);
}
.row.current .t {
  color: var(--accent);
}

.idx {
  width: 24px;
  flex: none;
  text-align: center;
  color: var(--text-2);
  font-variant-numeric: tabular-nums;
  display: flex;
  align-items: center;
  justify-content: center;
}
.idx .play-ico {
  display: none;
  width: 16px;
  height: 16px;
  color: var(--text);
}
.row:hover .idx .num {
  display: none;
}
.row:hover .idx .play-ico {
  display: block;
}

.cov {
  position: relative;
  width: 40px;
  height: 40px;
  flex: none;
  border-radius: 5px;
  overflow: hidden;
  background: var(--bg-3);
}
.cov img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}
.cov .ph {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-3);
  font-size: 16px;
}
.cov .play-ico {
  position: absolute;
  inset: 0;
  margin: auto;
  width: 18px;
  height: 18px;
  color: #fff;
  filter: drop-shadow(0 1px 2px rgba(0, 0, 0, 0.6));
  opacity: 0;
  transition: opacity 0.12s ease;
}
.row:hover .cov .play-ico {
  opacity: 1;
}

.main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}
.t {
  font-size: 13.5px;
  font-weight: 500;
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

.al {
  width: 26%;
  flex: none;
  font-size: 12.5px;
  color: var(--text-2);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.badge {
  flex: none;
  font-size: 10px;
  color: var(--accent);
  border: 1px solid var(--accent);
  border-radius: 4px;
  padding: 0 4px;
  line-height: 15px;
  opacity: 0.85;
}

.exp {
  flex: none;
  width: 26px;
  height: 26px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-3);
  opacity: 0;
  transition: opacity 0.2s ease, background 0.2s ease, color 0.2s ease;
}
.exp svg {
  width: 15px;
  height: 15px;
}
.row:hover .exp {
  opacity: 1;
}
.exp:hover {
  background: var(--hover);
  color: var(--text);
}
.del {
  flex: none;
  width: 26px;
  height: 26px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-3);
  opacity: 0;
  transition: opacity 0.2s ease, background 0.2s ease, color 0.2s ease;
}
.del svg {
  width: 15px;
  height: 15px;
}
.row:hover .del {
  opacity: 1;
}
.del:hover {
  background: var(--hover);
  color: var(--text);
}
.del.confirm {
  opacity: 1;
  background: var(--danger-bg);
  color: var(--danger);
}
.del.confirm:hover {
  background: var(--danger-bg);
  color: var(--danger);
}

.dur {
  flex: none;
  width: 40px;
  text-align: right;
  font-size: 12px;
  color: var(--text-2);
  font-variant-numeric: tabular-nums;
}

/* 多选状态 */
.row.picked {
  background: color-mix(in srgb, var(--accent) 10%, transparent);
}
.row.picked:hover {
  background: color-mix(in srgb, var(--accent) 16%, transparent);
}

/* 多选操作条 */
.sel-bar {
  position: sticky;
  bottom: 12px;
  margin-top: 14px;
  z-index: 20;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 8px 10px 8px 16px;
  border-radius: 999px;
  background: var(--pill-bg, rgba(0, 0, 0, 0.06));
  border: 1px solid var(--hairline);
  box-shadow: 0 8px 28px rgba(0, 0, 0, 0.16);
}
/* 边缘透镜折射:仅玻璃皮肤挂载(纸质/经典铬底下 sel-bar 为纸面材质) */
@supports (backdrop-filter: url("#glass-refract")) {
  html.glass .sel-bar {
    backdrop-filter: blur(20px) saturate(1.6) url("#glass-refract");
    -webkit-backdrop-filter: blur(20px) saturate(1.6) url("#glass-refract");
  }
}
.sel-bar.fixed {
  position: fixed;
  left: 50%;
  bottom: 16px;
  transform: translateX(-50%);
  width: min(560px, 86%);
  margin-top: 0;
  z-index: 50;
  box-shadow: 0 10px 34px rgba(0, 0, 0, 0.28);
}
.sel-count {
  font-size: 13px;
  font-weight: 700;
  color: var(--accent);
  white-space: nowrap;
}
.sel-actions {
  display: flex;
  align-items: center;
  gap: 6px;
}
.sel-btn {
  flex: none;
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text);
  background: transparent;
  border-radius: 999px;
  padding: 6px 14px;
  transition: background 0.18s ease, color 0.18s ease, transform 0.3s var(--ease-spring);
}
.sel-btn:hover {
  background: var(--hover);
}
.sel-btn:active {
  transform: scale(0.96);
  transition-duration: 0.08s;
}
.sel-btn.primary {
  background: var(--accent);
  color: #fff;
}
.sel-btn.primary:hover {
  background: var(--accent-hover);
}
.sel-btn.danger.confirm {
  color: var(--danger);
  border: 1px solid var(--danger);
}
.sel-btn.ghost {
  color: var(--text-2);
}

.bar-enter-active,
.bar-leave-active {
  transition: opacity 0.2s ease, transform 0.3s var(--ease-spring);
}
.bar-enter-from,
.bar-leave-to {
  opacity: 0;
  transform: translateY(14px);
}
</style>
