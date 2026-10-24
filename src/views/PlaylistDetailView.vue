<script setup lang="ts">
import { computed } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import { fmtTime, playTracks, shuffleList } from "../stores/player";
import { go, openAlbum, openArtist, ui } from "../stores/ui";
import { openCtx, type CtxItem } from "../stores/context";
import { exportTrackTmc } from "../stores/library";
import { useConfirmableAction } from "../composables/useConfirmableAction";
import {
  askRenamePlaylist,
  deletePlaylist,
  exportPlaylist,
  moveEntry,
  openPlaylistPicker,
  playlistById,
  removeEntry,
  resolvePlaylist,
  type ResolvedEntry,
} from "../stores/playlists";

const pl = computed(() => playlistById(ui.playlistId));
const resolved = computed(() => (pl.value ? resolvePlaylist(pl.value) : []));
const playable = computed(() => resolved.value.filter((r) => r.track).map((r) => r.track!));
const missing = computed(() => resolved.value.filter((r) => !r.track).length);

function playAll(): void {
  if (playable.value.length) playTracks(playable.value, 0);
}

function shuffleAll(): void {
  const list = playable.value;
  if (list.length === 0) return;
  shuffleList(list);
}

/** 删除列表不可恢复(与删歌不同,没有回收站兜底):两段确认防误触 */
const { confirmingId: confirmingDeletePl, confirm: confirmDeletePl } = useConfirmableAction();

function onDeletePlaylist(): void {
  if (!pl.value) return;
  if (confirmDeletePl("__playlist__")) void deletePlaylist(pl.value);
}

/** 菜单打开期间曲库可能被后台事件(library-changed)刷新,条目顺序与数量会变;
 *  执行时按 entry 引用重新定位,已不在列表里就放弃,不盲用打开菜单瞬间的下标 */
function entryIndexNow(r: ResolvedEntry): number {
  return pl.value ? pl.value.entries.indexOf(r.entry) : -1;
}

function onRemove(index: number): void {
  if (pl.value) void removeEntry(pl.value.id, index);
}

function onMove(index: number, delta: number): void {
  if (pl.value) void moveEntry(pl.value.id, index, index + delta);
}

/** 点击某行:以整个播放列表为队列,从该行开始播放 */
function playRow(index: number): void {
  const r = resolved.value[index];
  if (!r?.track) return;
  const pos = playable.value.findIndex((t) => t.id === r.track!.id);
  playTracks(playable.value, Math.max(0, pos));
}

/** 行聚焦时:Enter/Space 播放,Delete 对应右键菜单的"从列表移除"(不删文件)。
 *  行内上移/下移按钮聚焦时按键不冒泡,必须限定事件源是行自身 */
function onRowKey(e: KeyboardEvent, index: number): void {
  if (e.target !== e.currentTarget) return;
  if (e.key === "Enter" || e.key === " ") {
    e.preventDefault();
    playRow(index);
  } else if (e.key === "Delete") {
    e.preventDefault();
    onRemove(index);
  }
}

/** 歌单内歌曲右键菜单 */
function rowMenu(e: MouseEvent, index: number): void {
  const r = resolved.value[index];
  if (!r) return;
  const t = r.track;
  const items: CtxItem[] = [];
  if (t) {
    const playThis = () => {
      const i = entryIndexNow(r);
      if (i >= 0) playRow(i);
    };
    items.push(
      { label: "播放", icon: "play", action: playThis },
      {
        label: "查看专辑",
        icon: "album",
        action: () => openAlbum(`${t.albumArtist || t.artist}\u{1}${t.album}`),
      },
      { label: "查看艺人", icon: "artist", action: () => openArtist(t.artist) },
      { label: "加入其他歌单", icon: "playlist", action: () => openPlaylistPicker([t.id]) },
      {
        label: "导出为 TMC 音乐包",
        icon: "export",
        action: () => void exportTrackTmc(t.id, t.title, t.artist),
      },
    );
  }
  if (index > 0) {
    items.push({
      label: "上移",
      action: () => {
        const i = entryIndexNow(r);
        if (i > 0) onMove(i, -1);
      },
    });
  }
  if (pl.value && index < pl.value.entries.length - 1) {
    items.push({
      label: "下移",
      action: () => {
        const i = entryIndexNow(r);
        if (i >= 0 && i < pl.value!.entries.length - 1) onMove(i, 1);
      },
    });
  }
  items.push({
    label: "从列表移除(不删文件)",
    danger: true,
    action: () => {
      const i = entryIndexNow(r);
      if (i >= 0) onRemove(i);
    },
  });
  openCtx(e, items);
}
</script>

<template>
  <div v-if="pl" class="view">
    <button class="back-btn" @click="go('playlists')">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="m14.5 6-6 6 6 6" />
      </svg>
      播放列表
    </button>

    <header class="detail-head">
      <div class="cover">
        <img
          v-if="playable.length && playable[0].cover"
          :src="convertFileSrc(playable[0].cover)"
          alt=""
        />
        <div v-else class="ph">♪</div>
      </div>
      <div class="meta-col">
        <h1>{{ pl.name }}</h1>
        <div class="sub">
          {{ pl.entries.length }} 首<template v-if="missing"> · {{ missing }} 首文件缺失</template>
        </div>
        <div class="btns">
          <button class="pill pill--accent primary-pill" :disabled="!playable.length" @click="playAll">播放</button>
          <button class="pill ghost-pill" :disabled="!playable.length" @click="shuffleAll">随机播放</button>
          <button class="pill ghost-pill" title="导出为 TMCL(7z:音乐文件+playlist.json)" @click="exportPlaylist(pl)">
            导出 TMCL
          </button>
          <button class="pill ghost-pill" @click="askRenamePlaylist(pl)">重命名</button>
          <button
            class="pill ghost-pill danger"
            :class="{ confirm: confirmingDeletePl }"
            :title="confirmingDeletePl ? '再次点击确认删除' : '仅删除列表,不删除音乐文件(不可恢复)'"
            @click="onDeletePlaylist"
          >
            {{ confirmingDeletePl ? "确认删除?" : "删除列表" }}
          </button>
        </div>
      </div>
    </header>

    <div v-if="pl.entries.length === 0" class="empty">
      <h2>列表还是空的</h2>
      <p>在歌曲/专辑/搜索结果里右键选“添加到播放列表”,或勾选多首后点“加入歌单”</p>
    </div>

    <div v-else class="rows">
      <div
        v-for="r in resolved"
        :key="r.entry.path + '-' + r.index"
        class="row"
        :class="{ missing: !r.track }"
        role="button"
        tabindex="0"
        @click="playRow(r.index)"
        @contextmenu="rowMenu($event, r.index)"
        @keydown="onRowKey($event, r.index)"
      >
        <span class="idx">{{ r.index + 1 }}</span>
        <span class="cov">
          <img v-if="r.track?.cover" :src="convertFileSrc(r.track.cover)" loading="lazy" decoding="async" alt="" />
          <span v-else class="ph">♪</span>
        </span>
        <span class="main">
          <span class="t" :title="r.track?.title ?? r.entry.title">
            {{ r.track?.title ?? r.entry.title }}
            <span v-if="!r.track" class="miss-tag">文件缺失</span>
          </span>
          <span class="a">{{ r.track?.artist ?? r.entry.artist }}</span>
        </span>
        <span class="al" :title="r.track?.album ?? r.entry.album">{{ r.track?.album ?? r.entry.album }}</span>
        <span class="dur">{{ fmtTime(r.track?.duration ?? r.entry.duration) }}</span>
        <span class="ops" @click.stop>
          <button class="op" title="上移" :disabled="r.index === 0" @click="onMove(r.index, -1)">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="m6 14.5 6-6 6 6" />
            </svg>
          </button>
          <button
            class="op"
            title="下移"
            :disabled="r.index === pl.entries.length - 1"
            @click="onMove(r.index, 1)"
          >
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="m6 9.5 6 6 6-6" />
            </svg>
          </button>
          <!-- 移除只走右键菜单:行内 × 悬停浮现后位置固定,删一行行上移,
               × 会悬到下一首的同一位置,极易连环误删(歌单被清空事故的根源) -->
        </span>
      </div>
    </div>
  </div>

  <div v-else class="view">
    <div class="empty">
      <h2>未找到该播放列表</h2>
      <button class="pill pill--accent primary-pill" @click="go('playlists')">返回播放列表</button>
    </div>
  </div>
</template>

<style scoped>
.detail-head {
  display: flex;
  gap: 28px;
  align-items: flex-end;
  margin-bottom: 22px;
}
.cover {
  width: 190px;
  height: 190px;
  flex: none;
  border-radius: 10px;
  overflow: hidden;
  box-shadow: var(--card-shadow);
  background: var(--bg-3);
}
.cover img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}
.cover .ph {
  width: 100%;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 46px;
  color: var(--text-3);
}
.meta-col {
  min-width: 0;
}
.detail-head h1 {
  font-size: 34px;
  font-weight: 700;
  margin: 2px 0 4px;
  word-break: break-word;
}
.sub {
  font-size: 13px;
  color: var(--text-2);
  margin-bottom: 14px;
}
.btns {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
}
/* 幽灵胶囊:基类 .pill 之上只留尺寸与悬停亮度反馈(悬停不换底,
   保留"确认删除"实底切换不被悬停底色盖掉) */
.ghost-pill {
  padding: 8px 20px;
  font-size: 13.5px;
  transition: filter 0.2s ease, transform 0.35s var(--ease-spring);
}
.ghost-pill:hover {
  filter: brightness(1.06);
}
.ghost-pill.danger {
  color: var(--danger);
}
.ghost-pill.confirm {
  background: var(--danger);
  color: #fff;
}
/* 播放按钮的禁用(空列表)原样式不做淡化,基类的禁用变灰在此关掉 */
.primary-pill:disabled {
  opacity: 1;
  cursor: pointer;
}

.rows {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 7px 10px;
  border-radius: 8px;
  cursor: pointer;
  min-width: 0;
  transition: background 0.25s var(--ease-out-soft);
  /* TMCL 导入的大歌单轻松上千行:视口外的行跳过布局与绘制 */
  content-visibility: auto;
  contain-intrinsic-size: auto 54px;
}
.row:hover {
  background: var(--hover);
}
/* 键盘可达:焦点环对齐全局 accent 约定 */
.row:focus-visible {
  outline: 2px solid color-mix(in srgb, var(--accent) 45%, transparent);
  outline-offset: 2px;
}
.row.missing {
  opacity: 0.55;
  cursor: default;
}
.idx {
  width: 24px;
  flex: none;
  text-align: center;
  font-size: 12.5px;
  color: var(--text-2);
  font-variant-numeric: tabular-nums;
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
.miss-tag {
  font-size: 10.5px;
  color: var(--danger);
  border: 1px solid var(--danger);
  border-radius: 4px;
  padding: 0 4px;
  margin-left: 6px;
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
.dur {
  flex: none;
  width: 40px;
  text-align: right;
  font-size: 12px;
  color: var(--text-2);
  font-variant-numeric: tabular-nums;
}

.ops {
  display: flex;
  align-items: center;
  gap: 2px;
  flex: none;
  opacity: 0;
  transition: opacity 0.2s ease;
}
.row:hover .ops,
/* 悬停才现身的操作组:键盘焦点进入时同样现身 */
.row:focus-within .ops {
  opacity: 1;
}
.op {
  width: 26px;
  height: 26px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-2);
  transition: background 0.2s ease, color 0.2s ease;
}
.op:hover:not(:disabled) {
  background: var(--hover);
  color: var(--text);
}
.op:disabled {
  opacity: 0.35;
  cursor: default;
}
.op.danger:hover {
  color: var(--danger);
}
.op svg {
  width: 14px;
  height: 14px;
}
</style>
