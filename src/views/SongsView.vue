<script setup lang="ts">
import { allSongs, lib, enrichAllNetease } from "../stores/library";
import { shuffleList } from "../stores/player";
import TrackList from "../components/TrackList.vue";
import LibGate from "../components/LibGate.vue";

/** 全库随机播放:队列 = 全部歌曲洗牌,不打断正在播的歌 */
function shuffleAll(): void {
  const songs = allSongs.value;
  if (songs.length === 0) return;
  shuffleList(songs);
}
</script>

<template>
  <div class="view">
    <LibGate>
      <header class="view-head">
        <h1>歌曲</h1>
        <div class="right">
          <span class="count">{{ allSongs.length }} 首</span>
          <button class="match-btn ghost" :disabled="allSongs.length === 0" @click="shuffleAll">
            随机播放全部
          </button>
          <button class="match-btn" :disabled="lib.matching" @click="enrichAllNetease">
            {{ lib.matching ? "匹配中…" : "全部在线匹配" }}
          </button>
        </div>
      </header>
      <TrackList :tracks="allSongs" show-album show-cover virtual />
    </LibGate>
  </div>
</template>

<style scoped>
/* 虚拟滚动:视图变为有界容器,列表自身滚动 */
.view {
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.view :deep(.tl.virtual) {
  flex: 1;
  min-height: 0;
}
.view :deep(.view-head) {
  flex: none;
  margin-bottom: 14px;
}

.right {
  display: flex;
  align-items: center;
  gap: 12px;
}
.match-btn {
  font-size: 12.5px;
  font-weight: 600;
  color: #fff;
  background: var(--accent);
  border-radius: 999px;
  padding: 6px 16px;
  transition: background 0.2s ease, transform 0.35s var(--ease-spring), opacity 0.2s ease;
}
.match-btn:hover:not(:disabled) {
  background: var(--accent-hover);
}
.match-btn:active:not(:disabled) {
  transform: scale(0.97);
  transition-duration: 0.09s;
}
.match-btn.ghost {
  color: var(--text);
  background: var(--pill-bg);
}
.match-btn.ghost:hover:not(:disabled) {
  background: var(--hover);
}
.match-btn:disabled {
  opacity: 0.55;
  cursor: default;
}
</style>
