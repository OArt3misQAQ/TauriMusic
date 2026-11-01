<script setup lang="ts">
import { computed, ref } from "vue";
import { player, seek, fmtTime } from "../stores/player";

/* 进度条独立成组件:timeupdate(~4Hz)只重渲这里,不再把整个播放页
   (歌词逐行、队列抽屉)拖进每秒多次的 vnode diff */
/* 拖动中本地预览,松手(change)才真正 seek——
   input 每 tick 都写 audio.currentTime 会造成音频抖动 */
const seeking = ref<number | null>(null);
const seekShow = computed(() => seeking.value ?? player.position);
const progress = computed(() =>
  player.duration > 0 ? (seekShow.value / player.duration) * 100 : 0,
);
function onSeekInput(e: Event): void {
  seeking.value = Number((e.target as HTMLInputElement).value);
}
function onSeekCommit(e: Event): void {
  seek(Number((e.target as HTMLInputElement).value));
  seeking.value = null;
}
</script>

<template>
  <div class="seek">
    <span class="time">{{ fmtTime(seekShow) }}</span>
    <input
      class="range"
      type="range"
      min="0"
      :max="player.duration || 1"
      step="0.1"
      :value="seekShow"
      :style="{ '--fill': progress + '%' }"
      @input="onSeekInput"
      @change="onSeekCommit"
    />
    <span class="time">{{ fmtTime(player.duration) }}</span>
  </div>
</template>

<style scoped>
/* 样式随组件从 NowPlaying 原样搬入;皮肤变体在 style.css 末尾按 .seek/.range 命中 */
.seek {
  display: flex;
  align-items: center;
  gap: 12px;
  width: 560px;
  max-width: 90vw;
}
.time {
  font-size: 11px;
  color: var(--np-fg-soft);
  font-variant-numeric: tabular-nums;
  min-width: 36px;
}
.time:last-child {
  text-align: right;
}
.seek .range {
  flex: 1;
  background: linear-gradient(to right, var(--np-fg) var(--fill, 0%), var(--np-track) var(--fill, 0%));
}
.seek .range::-webkit-slider-thumb {
  background: var(--np-thumb);
}

/* 窄窗(与 NowPlaying 的断点一致):进度条拉满整行 */
@media (max-width: 860px) {
  .seek {
    width: 100%;
    max-width: none;
    padding: 0 22px;
  }
}
</style>
