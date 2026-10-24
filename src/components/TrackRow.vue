<script setup lang="ts">
import { convertFileSrc } from "@tauri-apps/api/core";
import type { Track } from "../api";
import { fmtTime } from "../stores/player";

/** 曲目行:TrackList 虚拟/非虚拟两分支共用的行标记(原两处约 90 行逐字重复)。
 *  组件保持无状态:选中/确认等状态由父级算好传入,事件全部上抛 */
defineProps<{
  track: Track;
  /** 显示序号(全库列表 / 专辑内曲目) */
  index: number;
  /** 是否当前播放行 */
  current?: boolean;
  /** 是否被多选勾中 */
  picked?: boolean;
  /** 虚拟滚动的回收态(RecycleScroller 的 active),仅作类名透传保持原行为 */
  hovered?: boolean;
  /** 虚拟模式固定行高(px);不传即内容自适应(非虚拟分支) */
  fixedHeight?: number;
  showIndex?: boolean;
  showCover?: boolean;
  showAlbum?: boolean;
  /** 删除按钮处于两段确认态 */
  confirming?: boolean;
}>();

const emit = defineEmits<{
  /** 键盘激活(Enter/Space):等同普通点击播放 */
  activate: [];
  rowClick: [e: MouseEvent];
  rowMenu: [e: MouseEvent];
  exportOne: [];
  deleteOne: [];
}>();

/** 行级快捷键:Enter/Space 播放,Delete 走右键菜单同款删除入口。
 *  行内按钮(导出/删除)聚焦时按键不冒泡成"播放",必须限定事件源是行自身 */
function onRowKey(e: KeyboardEvent): void {
  if (e.target !== e.currentTarget) return;
  if (e.key === "Enter" || e.key === " ") {
    e.preventDefault();
    emit("activate");
  } else if (e.key === "Delete") {
    e.preventDefault();
    emit("deleteOne");
  }
}
</script>

<template>
  <div
    class="row"
    :class="{ current, picked, hovered, virtual: fixedHeight != null }"
    :style="fixedHeight != null ? { height: fixedHeight + 'px' } : undefined"
    role="button"
    tabindex="0"
    @click="emit('rowClick', $event)"
    @contextmenu="emit('rowMenu', $event)"
    @keydown="onRowKey"
  >
    <span v-if="showIndex" class="idx">
      <span class="num">{{ track.trackNo ?? index + 1 }}</span>
      <svg class="play-ico" viewBox="0 0 24 24"><path d="M8.6 6.5v11L18 12z" fill="currentColor" /></svg>
    </span>
    <span v-if="showCover" class="cov">
      <img v-if="track.cover" :src="convertFileSrc(track.cover)" loading="lazy" decoding="async" alt="" />
      <span v-else class="ph">♪</span>
      <svg class="play-ico" viewBox="0 0 24 24"><path d="M8.6 6.5v11L18 12z" fill="currentColor" /></svg>
    </span>
    <span class="main">
      <span class="t" :title="track.title">{{ track.title }}</span>
      <span class="a" :title="track.artist">{{ track.artist }}</span>
    </span>
    <span v-if="showAlbum" class="al" :title="track.album">{{ track.album }}</span>
    <span v-if="track.hasLyrics" class="badge">词</span>
    <button class="exp" title="导出为 TMC 音乐包" @click.stop="emit('exportOne')">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <path d="M12 13.5V4M8.5 7.5 12 4l3.5 3.5" />
        <path d="M5 15v2.6c0 .77.63 1.4 1.4 1.4h11.2c.77 0 1.4-.63 1.4-1.4V15" />
      </svg>
    </button>
    <button
      class="del"
      :class="{ confirm: confirming }"
      :title="confirming ? '再次点击确认删除(移入回收站)' : '删除这首歌(移入回收站)'"
      @click.stop="emit('deleteOne')"
    >
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <path d="M4.5 7h15M9.5 7V5.2c0-.66.54-1.2 1.2-1.2h2.6c.66 0 1.2.54 1.2 1.2V7M6.5 7l.8 11.3c.06.77.7 1.2 1.4 1.2h6.6c.7 0 1.34-.43 1.4-1.2L17.5 7" />
        <path d="M10 11v5M14 11v5" />
      </svg>
    </button>
    <span class="dur">{{ fmtTime(track.duration) }}</span>
  </div>
</template>

<style scoped>
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
.row.virtual .t {
  line-height: 18px;
}
.row.virtual .a {
  line-height: 16px;
}
.row:hover {
  background: var(--hover);
}
.row.current .t {
  color: var(--accent);
}
/* 键盘可达:焦点环对齐全局 accent 约定(与在线搜索面板一致) */
.row:focus-visible {
  outline: 2px solid color-mix(in srgb, var(--accent) 45%, transparent);
  outline-offset: 2px;
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
/* 悬停才现身的按钮:键盘聚焦时同样现身,否则 Tab 走到看不见的按钮 */
.exp:focus-visible,
.del:focus-visible {
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
</style>
