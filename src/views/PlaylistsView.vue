<script setup lang="ts">
import { computed } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import { lib } from "../stores/library";
import { openPlaylist, ui } from "../stores/ui";
import { askCreatePlaylist, openPlaylistMenu, resolvePlaylist } from "../stores/playlists";
import LibGate from "../components/LibGate.vue";

const sorted = computed(() =>
  [...lib.playlists].sort((a, b) => b.createdAt - a.createdAt),
);

/** 卡片数据一次算完:拼贴封面与缺失数都要整单解析,放进模板函数的话
 *  每张卡片每次渲染要重复跑 4 遍 resolvePlaylist(全表 Map 查找+对象分配) */
const cards = computed(() =>
  sorted.value.map((p) => {
    let missing = 0;
    const covers: string[] = [];
    for (const r of resolvePlaylist(p)) {
      if (!r.track) {
        missing += 1;
      } else if (r.track.cover && covers.length < 4) {
        covers.push(convertFileSrc(r.track.cover));
      }
    }
    return { p, covers, missing, count: p.entries.length };
  }),
);
</script>

<template>
  <div class="view">
    <LibGate>
      <header class="view-head">
        <h1>播放列表</h1>
        <div class="right">
          <span class="count">{{ lib.playlists.length }} 个</span>
          <button class="pill pill--accent new-btn" @click="askCreatePlaylist">新建播放列表</button>
        </div>
      </header>

      <div v-if="sorted.length === 0" class="empty">
        <h2>还没有播放列表</h2>
        <p>把喜欢的歌收进自己的歌单,可自由命名、导出为 TMCL 分享或备份</p>
        <button class="pill pill--accent primary-pill" style="margin-top: 8px" @click="askCreatePlaylist">
          新建播放列表
        </button>
      </div>

      <div v-else class="grid">
        <div
          v-for="c in cards"
          :key="c.p.id"
          class="card"
          :class="{ active: ui.view === 'playlist' && ui.playlistId === c.p.id }"
          @click="openPlaylist(c.p.id)"
          @contextmenu="openPlaylistMenu($event, c.p)"
        >
          <div class="cover-collage">
            <template v-if="c.covers.length">
              <img
                v-for="(cover, i) in c.covers"
                :key="i"
                :src="cover"
                loading="lazy" decoding="async"
                :class="{ single: c.covers.length === 1 }"
                alt=""
              />
            </template>
            <div v-else class="ph">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round">
                <path d="M4 6.5h11M4 12h11M4 17.5h7" />
                <circle cx="18.5" cy="15.5" r="2.2" />
                <path d="M20.7 15.5V9l-3.4.9" />
              </svg>
            </div>
          </div>
          <div class="name" :title="c.p.name">{{ c.p.name }}</div>
          <div class="meta">
            {{ c.count }} 首<template v-if="c.missing"> · {{ c.missing }} 首缺失</template>
          </div>
        </div>
      </div>
    </LibGate>
  </div>
</template>

<style scoped>
.right {
  display: flex;
  align-items: center;
  gap: 14px;
}
/* 主行动胶囊:基类 .pill 之上只留尺寸与按压细节 */
.new-btn {
  padding: 6px 16px;
  font-size: 12.5px;
}
.new-btn:active {
  transform: scale(0.97);
  transition-duration: 0.09s;
}

.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(168px, 1fr));
  gap: 24px 20px;
}
.card {
  cursor: pointer;
  min-width: 0;
}
.cover-collage {
  position: relative;
  aspect-ratio: 1;
  border-radius: 8px;
  overflow: hidden;
  box-shadow: var(--card-shadow);
  background: var(--bg-3);
  display: grid;
  grid-template-columns: 1fr 1fr;
  grid-template-rows: 1fr 1fr;
  gap: 1px;
  transition: transform 0.45s var(--ease-spring);
}
.card:hover .cover-collage {
  transform: translateY(-4px) scale(1.015);
}
.cover-collage img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}
.cover-collage img.single {
  grid-column: 1 / 3;
  grid-row: 1 / 3;
}
.cover-collage .ph {
  grid-column: 1 / 3;
  grid-row: 1 / 3;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-3);
}
.cover-collage .ph svg {
  width: 42px;
  height: 42px;
}
.card.active .cover-collage {
  box-shadow: 0 0 0 2px var(--accent), var(--card-shadow);
}

.name {
  margin-top: 10px;
  font-size: 13px;
  font-weight: 600;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.meta {
  margin-top: 2px;
  font-size: 12px;
  color: var(--text-2);
}
</style>
