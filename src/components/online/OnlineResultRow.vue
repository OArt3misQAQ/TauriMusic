<script setup lang="ts">
import { computed } from "vue";
import { onlineTrackId, type OnlineTrack } from "../../api";
import { current, fmtTime } from "../../stores/player";
import { openCtx, type CtxItem } from "../../stores/context";
import { online, playable, play, save } from "../../stores/online";

const props = defineProps<{ t: OnlineTrack; i: number }>();

/** 正在播放高亮:与播放队列 id(来源前缀+平台 id,同 onlineToTrack 的拼法)比对 */
const isCurrent = computed(() => current.value?.id === onlineTrackId(props.t));

/** 行右键菜单:闭包捕获搜索结果里的在线曲目,与本地曲目共用同一套菜单架构;
 *  保存项与行内保存按钮同口径,仅免费曲目出现 */
function rowMenu(e: MouseEvent): void {
  const items: CtxItem[] = [{ label: "播放", icon: "play", action: () => play(props.t) }];
  if (playable(props.t)) {
    items.push({ label: "保存到本地库", icon: "save", action: () => void save(props.t) });
  }
  openCtx(e, items);
}
</script>

<template>
  <div
    class="ov-row"
    :class="{ dim: !playable(t), saving: online.savingId === t.id, current: isCurrent }"
    @contextmenu="rowMenu"
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
      :disabled="online.savingId !== ''"
      :aria-label="`保存 ${t.name} 到本地库`"
      :title="online.savingId === t.id ? '正在下载…' : '保存到本地库'"
      @click="save(t)"
    >
      <span v-if="online.savingId === t.id" class="row-spin" aria-hidden="true"></span>
      <svg v-else viewBox="0 0 24 24" aria-hidden="true">
        <path d="M12 4.6v9.2m0 0 3.4-3.4M12 13.8l-3.4-3.4" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
        <path d="M5.4 18.4h13.2" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
      </svg>
    </button>
  </div>
</template>

<style scoped>
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
/* 当前播放:与 TrackList 的 row.current 同一令牌,只染标题色 */
.ov-row.current .t {
  color: var(--accent);
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
/* 保存按钮:行 hover 才现身,和行内播放图标一个脾气;正在保存的那行常驻并转圈。
   用 opacity+visibility 过渡而非 display 硬切(对齐 TrackList 行内按钮),
   且按钮常占布局位,hover 时时长数字不再左右跳动 */
.row-save {
  flex: none;
  width: 32px;
  height: 32px;
  margin-right: 8px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-2);
  opacity: 0;
  visibility: hidden;
  transition:
    opacity 0.2s ease,
    visibility 0.2s ease,
    background 0.2s ease,
    color 0.2s ease;
}
.ov-row:hover .row-save,
.row-save:focus-visible,
.ov-row.saving .row-save {
  opacity: 1;
  visibility: visible;
}
/* 保存进行中按钮全体禁用:半透明示不可点。置于现身规则之后,同特异性时靠源序
   压过 hover/saving 的 opacity:1,保持旧版禁用态 0.6 的观感 */
.ov-row .row-save:disabled {
  opacity: 0.6;
  cursor: default;
}
.row-save:hover:not(:disabled) {
  background: var(--hover);
  color: var(--accent);
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
@keyframes ov-spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
