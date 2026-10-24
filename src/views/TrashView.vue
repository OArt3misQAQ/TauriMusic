<script setup lang="ts">
import { computed } from "vue";
import { lib, restoreTracks, purgeTrash } from "../stores/library";
import { fmtTime } from "../stores/player";
import { useConfirmableAction } from "../composables/useConfirmableAction";
import type { TrashEntry } from "../api";

/** 最新删除的排前面 */
const entries = computed(() => [...lib.trash].sort((a, b) => b.deletedAt - a.deletedAt));

const dateOf = (ts: number): string => new Date(ts * 1000).toLocaleString();

const { confirmingId, confirm } = useConfirmableAction();

function onRestore(e: TrashEntry): void {
  void restoreTracks([e.track.id]);
}

function onRestoreAll(): void {
  void restoreTracks([]);
}

/** 彻底删除采用两段确认:第一次点进入确认态,3 秒内再点执行 */
function onPurge(e: TrashEntry): void {
  if (confirm(e.track.id)) void purgeTrash([e.track.id]);
}

function onEmpty(): void {
  if (confirm("__empty__")) void purgeTrash([]);
}

/** 行聚焦时:Enter/Space 还原(行的主操作),Delete 彻底删除(两段确认) */
function onRowKey(e: KeyboardEvent, entry: TrashEntry): void {
  if (e.target !== e.currentTarget) return;
  if (e.key === "Enter" || e.key === " ") {
    e.preventDefault();
    onRestore(entry);
  } else if (e.key === "Delete") {
    e.preventDefault();
    onPurge(entry);
  }
}
</script>

<template>
  <div class="view">
    <header class="view-head">
      <h1>回收站</h1>
      <div class="right">
        <span class="count">{{ entries.length }} 首 · 保留 30 天</span>
        <button v-if="entries.length" class="pill pill--hover ghost-pill" @click="onRestoreAll">全部还原</button>
        <button
          v-if="entries.length"
          class="pill pill--hover ghost-pill danger"
          :title="confirmingId === '__empty__' ? '再次点击确认清空(不可恢复)' : '清空回收站'"
          @click="onEmpty"
        >
          {{ confirmingId === "__empty__" ? "确认清空?" : "清空回收站" }}
        </button>
      </div>
    </header>

    <div v-if="entries.length === 0" class="empty slim">
      <svg class="icon" width="64" height="64" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round">
        <path d="M4.5 7h15M9.5 7V5.2c0-.66.54-1.2 1.2-1.2h2.6c.66 0 1.2.54 1.2 1.2V7M6.5 7l.8 11.3c.06.77.7 1.2 1.4 1.2h6.6c.7 0 1.34-.43 1.4-1.2L18.5 7" />
        <path d="M10 11v5M14 11v5" />
      </svg>
      <h2>回收站是空的</h2>
      <p>删除的音乐会在这里保留 30 天,期间可随时还原</p>
    </div>

    <div v-else class="list">
      <div
        v-for="e in entries"
        :key="e.track.id"
        class="row"
        role="button"
        tabindex="0"
        @keydown="onRowKey($event, e)"
      >
        <div class="main">
          <div class="t" :title="e.track.title">{{ e.track.title }}</div>
          <div class="a">
            {{ e.track.artist }}<template v-if="e.track.album"> · {{ e.track.album }}</template>
          </div>
        </div>
        <span class="when" :title="dateOf(e.deletedAt)">删除于 {{ dateOf(e.deletedAt) }}</span>
        <span class="dur">{{ fmtTime(e.track.duration) }}</span>
        <button class="pill restore" @click="onRestore(e)">还原</button>
        <button
          class="pill del"
          :title="confirmingId === e.track.id ? '再次点击确认彻底删除(不可恢复)' : '彻底删除(不可恢复)'"
          @click="onPurge(e)"
        >
          {{ confirmingId === e.track.id ? "确认?" : "彻底删除" }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.right {
  display: flex;
  align-items: center;
  gap: 10px;
}
/* 幽灵胶囊:基类 .pill 之上只留尺寸,悬停换底走 .pill--hover */
.ghost-pill {
  padding: 6px 16px;
  font-size: 12.5px;
}
.ghost-pill.danger:hover {
  background: var(--active);
  color: var(--accent);
}

/* 紧凑空态(.empty.slim)之上再收间距:用顶部留白代替整屏高度 */
.empty.slim {
  gap: 4px;
  height: auto;
  padding-top: 16vh;
  text-align: center;
}
.empty.slim .icon {
  margin-bottom: 10px;
}
.empty.slim p {
  font-size: 13px;
}

.list {
  display: flex;
  flex-direction: column;
  max-width: 860px;
}
.row {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 9px 10px;
  border-radius: 8px;
}
.row:hover {
  background: var(--hover);
}
/* 键盘可达:焦点环对齐全局 accent 约定(行上 Enter=还原,Delete=彻底删除) */
.row:focus-visible {
  outline: 2px solid color-mix(in srgb, var(--accent) 45%, transparent);
  outline-offset: 2px;
}
.main {
  flex: 1;
  min-width: 0;
}
.t {
  font-size: 13.5px;
  font-weight: 600;
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
.when {
  font-size: 11.5px;
  color: var(--text-3);
  flex: none;
}
.dur {
  font-size: 12px;
  color: var(--text-2);
  flex: none;
  font-variant-numeric: tabular-nums;
}
/* 还原/彻底删除:基类 .pill 之上只留尺寸与专属配色(圆角/字重/底色走基类) */
.pill {
  flex: none;
  font-size: 12px;
  padding: 5px 13px;
  transition: background 0.2s ease, color 0.2s ease, transform 0.35s var(--ease-spring);
}
.pill:active {
  transform: scale(0.96);
  transition-duration: 0.09s;
}
.pill.restore {
  color: var(--accent);
  background: var(--pill-bg);
}
.pill.restore:hover {
  background: var(--active);
}
.pill.del {
  color: var(--text-2);
  background: var(--pill-bg);
}
.pill.del:hover {
  color: var(--accent);
  background: var(--active);
}
</style>
