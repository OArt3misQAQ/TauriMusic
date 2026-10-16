<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import type { Track } from "../api";
import { albums, artists, lib, type Album } from "../stores/library";
import { openAlbum, openArtist, ui } from "../stores/ui";
import TrackList from "../components/TrackList.vue";
import AlbumCard from "../components/AlbumCard.vue";
import LibGate from "../components/LibGate.vue";

/** 搜索结果渲染上限:命中数千首时全量渲染 DOM 会卡顿吃内存,超出部分提示细化关键词 */
const SEARCH_LIMIT = 500;

/** 键入防抖:大库全量匹配有成本,连续输入时合并成停顿后的一次查询 */
const debouncedSearch = ref("");
let searchTimer = 0;
watch(
  () => ui.search,
  (v) => {
    window.clearTimeout(searchTimer);
    searchTimer = window.setTimeout(() => (debouncedSearch.value = v), 160);
  },
  { immediate: true },
);
onUnmounted(() => window.clearTimeout(searchTimer));

const query = computed(() => debouncedSearch.value.trim().toLowerCase());

/** 预建小写索引:曲库变化才重建,每次键入不再对全库逐首 toLowerCase(每次调用都分配新串) */
const searchIndex = computed(() =>
  lib.tracks.map((t) => ({
    t,
    title: t.title.toLowerCase(),
    artist: t.artist.toLowerCase(),
    album: t.album.toLowerCase(),
  })),
);

const songsAll = computed<Track[]>(() => {
  const q = query.value;
  if (!q) return [];
  const out: Track[] = [];
  for (const it of searchIndex.value) {
    if (it.title.includes(q) || it.artist.includes(q) || it.album.includes(q)) out.push(it.t);
  }
  return out;
});
const songs = computed<Track[]>(() => songsAll.value.slice(0, SEARCH_LIMIT));

const matchedAlbums = computed<Album[]>(() => {
  const q = query.value;
  if (!q) return [];
  return albums.value.filter(
    (a) => a.album.toLowerCase().includes(q) || a.artist.toLowerCase().includes(q),
  );
});

const matchedArtists = computed<string[]>(() => {
  const q = query.value;
  if (!q) return [];
  return artists.value.filter((a) => a.toLowerCase().includes(q));
});

const nothing = computed(
  () =>
    query.value &&
    songs.value.length === 0 &&
    matchedAlbums.value.length === 0 &&
    matchedArtists.value.length === 0,
);
</script>

<template>
  <div class="view">
    <LibGate>
      <template v-if="query">
      <section v-if="songs.length" class="section">
        <h2>歌曲</h2>
        <p v-if="songsAll.length > SEARCH_LIMIT" class="limit-hint">
          共 {{ songsAll.length }} 条命中,已显示前 {{ SEARCH_LIMIT }} 条,请细化关键词
        </p>
        <TrackList :tracks="songs" show-album show-cover />
      </section>

      <section v-if="matchedAlbums.length" class="section">
        <h2>专辑</h2>
        <div class="grid">
          <AlbumCard v-for="a in matchedAlbums" :key="a.key" :album="a" @open="openAlbum(a.key)" />
        </div>
      </section>

      <section v-if="matchedArtists.length" class="section">
        <h2>艺人</h2>
        <div class="artist-grid">
          <button v-for="a in matchedArtists" :key="a" class="artist-chip" @click="openArtist(a)">
            <span class="avatar">{{ a.slice(0, 1) }}</span>
            {{ a }}
          </button>
        </div>
      </section>

      <div v-if="nothing" class="empty">
        <h2>没有找到“{{ ui.search }}”相关内容</h2>
      </div>
      </template>

      <div v-else class="empty">
        <h2>搜索你的音乐</h2>
        <p>按歌曲、专辑或艺人名称搜索</p>
      </div>
    </LibGate>
  </div>
</template>

<style scoped>
.limit-hint {
  font-size: 12px;
  color: var(--text-3);
  margin-bottom: 10px;
}
.section {
  margin-bottom: 30px;
}
.section h2 {
  font-size: 17px;
  font-weight: 700;
  margin-bottom: 12px;
}
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
  gap: 20px 18px;
}
.artist-grid {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
}
.artist-chip {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  padding: 5px 12px 5px 5px;
  border-radius: 999px;
  background: var(--pill-bg);
  font-size: 13px;
}
.artist-chip:hover {
  filter: brightness(1.05);
  background: var(--active);
}
.avatar {
  width: 26px;
  height: 26px;
  border-radius: 50%;
  background: var(--accent);
  color: #fff;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 12px;
  font-weight: 700;
}
</style>
