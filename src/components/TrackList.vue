<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { RecycleScroller } from "vue-virtual-scroller";
import type { Track } from "../api";
import { current, playTracks, playTrack } from "../stores/player";
import { exportTrackTmc, exportTracksTmc, deleteTracks, trackAlbumKey } from "../stores/library";
import { useConfirmableAction } from "../composables/useConfirmableAction";
import { openPlaylistPicker } from "../stores/playlists";
import { openAlbum, openArtist } from "../stores/ui";
import { openCtx } from "../stores/context";
import TrackRow from "./TrackRow.vue";

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
      <TrackRow
        :track="t"
        :index="i"
        :fixed-height="rowHeight"
        :hovered="active"
        :current="t.id === current?.id"
        :picked="selected.has(t.id)"
        :confirming="confirmingId === t.id"
        :show-index="showIndex"
        :show-cover="showCover"
        :show-album="showAlbum"
        @activate="play(t)"
        @row-click="rowClick($event, t, i)"
        @row-menu="rowMenu($event, t)"
        @export-one="exportOne(t)"
        @delete-one="onDelete(t)"
      />
    </template>
  </RecycleScroller>

  <div v-else class="tl">
    <TrackRow
      v-for="(t, i) in tracks"
      :key="t.id"
      :track="t"
      :index="i"
      :current="t.id === current?.id"
      :picked="selected.has(t.id)"
      :confirming="confirmingId === t.id"
      :show-index="showIndex"
      :show-cover="showCover"
      :show-album="showAlbum"
      @activate="play(t)"
      @row-click="rowClick($event, t, i)"
      @row-menu="rowMenu($event, t)"
      @export-one="exportOne(t)"
      @delete-one="onDelete(t)"
    />
  </div>

  <!-- 多选操作条(虚拟模式下为固定浮层) -->
  <Transition name="bar">
    <div v-if="selected.size > 0" class="sel-bar" :class="{ fixed: virtual }">
        <span class="sel-count">已选 {{ selected.size }} 首</span>
        <div class="sel-actions">
          <button class="pill pill--accent sel-btn" @click="playSelected">播放</button>
          <button class="pill pill--flat sel-btn" @click="openPlaylistPicker([...selected])">加入歌单</button>
          <button class="pill pill--flat sel-btn" @click="exportSelected">导出 TMC</button>
          <button class="pill pill--flat sel-btn danger" :class="{ confirm: confirmingBatchDelete }" @click="deleteSelected">
            {{ confirmingBatchDelete ? "确认删除?" : "删除" }}
          </button>
          <button class="pill pill--flat sel-btn ghost" @click="clearSelection">取消</button>
        </div>
      </div>
  </Transition>
</template>

<style scoped>
.tl {
  display: flex;
  flex-direction: column;
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
/* 选择条按钮:透明底胶囊,反馈靠悬停换底;确认态描边警示 */
.sel-btn {
  flex: none;
  font-size: 12.5px;
  padding: 6px 14px;
  transition: background 0.18s ease, color 0.18s ease, transform 0.3s var(--ease-spring);
}
.sel-btn:active {
  transition-duration: 0.08s;
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
