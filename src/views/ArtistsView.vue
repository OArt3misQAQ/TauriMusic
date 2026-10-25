<script setup lang="ts">
import { computed } from "vue";
import { artists, albums, lib, zhCollator } from "../stores/library";
import { openAlbum, ui } from "../stores/ui";
import { playTracks } from "../stores/player";
import { isMobile } from "../platform";
import AlbumCard from "../components/AlbumCard.vue";
import TrackList from "../components/TrackList.vue";
import LibGate from "../components/LibGate.vue";

const selected = computed(() => {
  if (ui.artist && artists.value.includes(ui.artist)) return ui.artist;
  return artists.value[0] ?? null;
});

const artistAlbums = computed(() => {
  const name = selected.value;
  if (!name) return [];
  return albums.value
    .filter((a) => a.tracks.some((t) => (t.artist || "未知艺人") === name))
    .sort((a, b) => zhCollator.compare(a.album, b.album));
});

const artistSongs = computed(() => {
  const name = selected.value;
  if (!name) return [];
  return lib.tracks
    .filter((t) => (t.artist || "未知艺人") === name)
    .sort(
      (a, b) =>
        zhCollator.compare(a.album, b.album) ||
        (a.discNo ?? 1) - (b.discNo ?? 1) ||
        (a.trackNo ?? 0) - (b.trackNo ?? 0),
    );
});
</script>

<template>
  <div class="view artists" :class="{ 'has-selected': !!ui.artist }">
    <LibGate>
      <aside class="list">
        <header class="view-head"><h1>艺人</h1></header>
        <button
          v-for="name in artists"
          :key="name"
          class="artist-item"
          :class="{ active: name === (isMobile ? ui.artist : selected) }"
          @click="ui.artist = name"
        >
          {{ name }}
        </button>
      </aside>

      <section v-if="selected" class="detail">
        <button class="mobile-back" @click="ui.artist = null">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="m14.5 6-6 6 6 6" />
          </svg>
          全部艺人
        </button>
        <header class="view-head">
          <h1>{{ selected }}</h1>
          <span class="count">{{ artistAlbums.length }} 张专辑 · {{ artistSongs.length }} 首歌曲</span>
        </header>

        <div v-if="artistAlbums.length" class="grid">
          <AlbumCard v-for="a in artistAlbums" :key="a.key" :album="a" @open="openAlbum(a.key)" />
        </div>

        <template v-if="artistSongs.length">
          <div class="sub-label">
            歌曲
            <button class="playall" @click="playTracks(artistSongs, 0)">全部播放</button>
          </div>
          <TrackList :tracks="artistSongs" show-album show-cover />
        </template>
      </section>

      <section v-else class="empty" style="width: 100%">
        <h2>曲库中还没有艺人</h2>
      </section>
    </LibGate>
  </div>
</template>

<style scoped>
.artists {
  display: flex;
  gap: 34px;
}

.list {
  width: 190px;
  flex: none;
  overflow-y: auto;
  border-right: 1px solid var(--hairline);
  padding-right: 16px;
  min-height: 0;
}
.list .view-head {
  margin-bottom: 10px;
}
.artist-item {
  display: block;
  width: 100%;
  text-align: left;
  padding: 7px 10px;
  border-radius: 7px;
  font-size: 13.5px;
  color: var(--text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.artist-item:hover {
  background: var(--hover);
}
.artist-item.active {
  background: var(--active);
  color: var(--accent);
  font-weight: 600;
}

.detail {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
  min-height: 0;
  padding-bottom: 40px;
}
.detail .view-head {
  margin-bottom: 14px;
}

.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
  gap: 20px 18px;
  margin-bottom: 30px;
}

.sub-label {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 16px;
  font-weight: 700;
  margin: 4px 0 12px;
}
.playall {
  color: var(--accent);
  font-size: 13px;
  font-weight: 600;
}
.playall:hover {
  text-decoration: underline;
}

.mobile-back {
  display: none;
  align-items: center;
  gap: 3px;
  height: 32px;
  margin-bottom: 8px;
  padding: 0 10px 0 6px;
  border-radius: 999px;
  background: var(--pill-bg);
  color: var(--text-2);
  font-size: 12.5px;
  font-weight: 600;
}
.mobile-back svg {
  width: 16px;
  height: 16px;
}

/* 手机:艺人列表与详情改成单列切换,不再左右分栏 */
:root.mobile .artists {
  display: block;
}
:root.mobile .artists .list {
  width: auto;
  border-right: 0;
  padding-right: 0;
  overflow: visible;
}
:root.mobile .artists .list .view-head {
  display: none;
}
:root.mobile .artists .artist-item {
  padding: 11px 12px;
  border-radius: 10px;
  font-size: 14px;
}
:root.mobile .artists.has-selected .list {
  display: none;
}
/* 桌面默认展示第一位艺人的详情;手机在未显式选人时只显示列表 */
:root.mobile .artists:not(.has-selected) .detail {
  display: none;
}
:root.mobile .artists .detail {
  width: 100%;
  overflow: visible;
  padding-bottom: 20px;
}
:root.mobile .mobile-back {
  display: inline-flex;
}
</style>
