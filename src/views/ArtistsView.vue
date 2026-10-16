<script setup lang="ts">
import { computed } from "vue";
import { artists, albums, lib, zhCollator } from "../stores/library";
import { openAlbum, ui } from "../stores/ui";
import { playTracks } from "../stores/player";
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
  <div class="view artists">
    <LibGate>
      <aside class="list">
        <header class="view-head"><h1>艺人</h1></header>
        <button
          v-for="name in artists"
          :key="name"
          class="artist-item"
          :class="{ active: name === selected }"
          @click="ui.artist = name"
        >
          {{ name }}
        </button>
      </aside>

      <section v-if="selected" class="detail">
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
</style>
