<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { go, ui, type ViewName } from "../stores/ui";

/** 搜索前的资料库子页:清空搜索后回到原处,而不是固定落回歌曲页 */
const viewBeforeSearch = ref<ViewName>("songs");
const searchInput = ref(ui.search);
let searchTimer = 0;

watch(searchInput, (v) => {
  window.clearTimeout(searchTimer);
  searchTimer = window.setTimeout(() => {
    if (v.trim()) {
      if (ui.view !== "search") viewBeforeSearch.value = ui.view;
      ui.view = "search";
    } else if (ui.view === "search") {
      ui.view = viewBeforeSearch.value;
    }
    ui.search = v;
  }, 180);
});

watch(
  () => ui.search,
  (v) => {
    if (v === "" && searchInput.value !== "") searchInput.value = "";
  },
);

onUnmounted(() => window.clearTimeout(searchTimer));

const chips = [
  { view: "songs", label: "歌曲" },
  { view: "albums", label: "专辑" },
  { view: "artists", label: "艺人" },
  { view: "playlists", label: "歌单" },
] as const;

/** 切换分类时清掉艺人详情选择:从"艺人"胶囊进入应看到列表,而不是上次的详情 */
function pick(view: ViewName): void {
  ui.artist = null;
  go(view);
}

/** 详情页也高亮所属分类;搜索页沿用进入搜索前的分类 */
const activeChip = computed<ViewName>(() => {
  if (ui.view === "search") return viewBeforeSearch.value;
  if (ui.view === "album") return "albums";
  if (ui.view === "playlist") return "playlists";
  return ui.view;
});
</script>

<template>
  <div class="lib-bar">
    <div class="lib-search">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round">
        <circle cx="11" cy="11" r="6.5" />
        <path d="m16.2 16.2 4.3 4.3" />
      </svg>
      <input
        v-model="searchInput"
        type="search"
        placeholder="搜索资料库"
        enterkeyhint="search"
        spellcheck="false"
      />
      <button v-if="searchInput" class="clear" title="清空" @click="searchInput = ''">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round">
          <path d="m7 7 10 10M17 7 7 17" />
        </svg>
      </button>
    </div>

    <div class="chips">
      <button
        v-for="c in chips"
        :key="c.view"
        class="chip"
        :class="{ on: activeChip === c.view }"
        @click="pick(c.view)"
      >
        {{ c.label }}
      </button>
    </div>
  </div>
</template>

<style scoped>
.lib-bar {
  flex: none;
  padding: 8px 16px 10px;
  border-bottom: 1px solid var(--hairline);
  background: color-mix(in srgb, var(--bg) 88%, transparent);
  backdrop-filter: blur(20px) saturate(1.5);
  -webkit-backdrop-filter: blur(20px) saturate(1.5);
  z-index: 20;
}
.lib-search {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 38px;
  padding: 0 12px;
  border-radius: 12px;
  background: var(--pill-bg);
}
.lib-search svg {
  width: 17px;
  height: 17px;
  flex: none;
  color: var(--text-3);
}
.lib-search input {
  flex: 1;
  min-width: 0;
  border: 0;
  outline: none;
  background: transparent;
  color: var(--text);
  font-size: 14.5px;
}
.lib-search input::placeholder {
  color: var(--text-3);
}
.lib-search input::-webkit-search-cancel-button {
  display: none;
}
.clear {
  flex: none;
  width: 26px;
  height: 26px;
  margin-right: -4px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-3);
}
.clear svg {
  width: 12px;
  height: 12px;
}
.clear:active {
  background: var(--hover);
}
.chips {
  display: flex;
  gap: 8px;
  margin-top: 8px;
  overflow-x: auto;
  scrollbar-width: none;
}
.chips::-webkit-scrollbar {
  display: none;
}
.chip {
  flex: none;
  height: 30px;
  padding: 0 15px;
  border-radius: 999px;
  border: 1px solid var(--hairline);
  background: transparent;
  color: var(--text-2);
  font-size: 12.5px;
  font-weight: 600;
  transition: background 0.18s ease, color 0.18s ease, transform 0.2s var(--ease-spring);
}
.chip.on {
  background: var(--accent);
  border-color: transparent;
  color: #fff;
}
.chip:active {
  transform: scale(0.95);
  transition-duration: 0.08s;
}
</style>
