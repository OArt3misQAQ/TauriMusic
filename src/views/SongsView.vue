<script setup lang="ts">
import { allSongs, lib, enrichAllNetease } from "../stores/library";
import { shuffleList } from "../stores/player";
import TrackList from "../components/TrackList.vue";
import LibGate from "../components/LibGate.vue";
import { isMobile } from "../platform";

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
          <button class="pill match-btn ghost" :disabled="allSongs.length === 0" @click="shuffleAll">
            {{ isMobile ? "随机播放" : "随机播放全部" }}
          </button>
          <!-- 全库在线匹配是桌面维护动作,手机端不占首屏 -->
          <button
            v-if="!isMobile"
            class="pill pill--accent match-btn"
            :disabled="lib.matching"
            @click="enrichAllNetease"
          >
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
/* 主行动胶囊:基类 .pill 之上只留尺寸与按压/禁用细节(底色/圆角/字重走基类) */
.match-btn {
  padding: 6px 16px;
  font-size: 12.5px;
  transition: background 0.2s ease, transform 0.35s var(--ease-spring), opacity 0.2s ease;
}
.match-btn:hover:not(:disabled) {
  background: var(--accent-hover);
}
.match-btn.ghost:hover:not(:disabled) {
  background: var(--hover);
}
/* 禁用态不换悬停底色(基类变体未区分禁用),与原样式一致;ghost 无 accent 底不受影响 */
.match-btn:not(.ghost):disabled:hover {
  background: var(--accent);
}
.match-btn:active:not(:disabled) {
  transform: scale(0.97);
  transition-duration: 0.09s;
}
.match-btn:disabled {
  opacity: 0.55;
  cursor: default;
}

/* 手机:分类由顶部胶囊承担,隐藏重复的"歌曲"大标题,只留计数与随机播放 */
:root.mobile .view-head h1 {
  display: none;
}
:root.mobile .view-head {
  justify-content: flex-end;
  margin-bottom: 10px;
}
</style>
