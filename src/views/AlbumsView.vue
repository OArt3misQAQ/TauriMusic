<script setup lang="ts">
import { computed } from "vue";
import { albums, zhCollator } from "../stores/library";
import { openAlbum } from "../stores/ui";
import AlbumCard from "../components/AlbumCard.vue";
import LibGate from "../components/LibGate.vue";

// 预建 collator 复用:裸 localeCompare 每次调用都重新解析 locale 选项,大列表明显更慢
const sorted = computed(() =>
  [...albums.value].sort(
    (a, b) => zhCollator.compare(a.artist, b.artist) || zhCollator.compare(a.album, b.album),
  ),
);
</script>

<template>
  <div class="view">
    <LibGate>
      <header class="view-head">
        <h1>专辑</h1>
        <span class="count">{{ sorted.length }} 张</span>
      </header>
      <div class="grid">
        <AlbumCard v-for="a in sorted" :key="a.key" :album="a" @open="openAlbum(a.key)" />
      </div>
    </LibGate>
  </div>
</template>

<style scoped>
.grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(168px, 1fr));
  gap: 24px 20px;
}
</style>
