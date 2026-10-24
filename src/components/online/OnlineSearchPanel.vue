<script setup lang="ts">
import { SOURCES, online, selectSource, search } from "../../stores/online";
</script>

<template>
  <section class="search-panel">
    <div class="panel-top">
      <div class="sources" role="tablist" aria-label="在线音源">
        <button
          v-for="s in SOURCES"
          :key="s.id"
          type="button"
          role="tab"
          :aria-selected="online.source === s.id"
          class="src"
          :class="{ on: online.source === s.id }"
          @click="selectSource(s.id)"
        >
          {{ s.label }}
        </button>
      </div>
      <span class="panel-hint">仅免费曲目 · 无需登录</span>
    </div>

    <!-- 回车提交交给 form:有 submit 按钮的表单里 input 回车天然触发 submit,不必再挂 keydown -->
    <form class="ov-search" @submit.prevent="search">
      <svg class="s-ico" viewBox="0 0 24 24" aria-hidden="true">
        <circle cx="11" cy="11" r="6.5" fill="none" stroke="currentColor" stroke-width="1.8" />
        <path d="m16 16 4.5 4.5" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
      </svg>
      <input
        v-model="online.keyword"
        type="search"
        enterkeyhint="search"
        aria-label="搜索在线音乐"
        placeholder="搜索歌曲 / 艺人 / 专辑"
      />
      <button v-if="online.keyword" class="clear" type="button" title="清空" @click="online.keyword = ''">
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="m7 7 10 10M17 7 7 17" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
        </svg>
      </button>
      <button class="go" type="submit" :disabled="!online.keyword.trim() || online.phase === 'loading'">
        {{ online.phase === "loading" && online.results.length === 0 ? "搜索中…" : "搜索" }}
      </button>
    </form>
  </section>
</template>

<style scoped>
.search-panel {
  padding: 14px;
  border-radius: 18px;
  border: 1px solid var(--hairline);
  background: color-mix(in srgb, var(--bg-2) 68%, transparent);
  box-shadow: 0 18px 48px rgba(0, 0, 0, 0.1);
  backdrop-filter: blur(16px);
  -webkit-backdrop-filter: blur(16px);
}
.panel-top {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}
.panel-hint {
  flex: none;
  font-size: 11.5px;
  color: var(--text-3);
  white-space: nowrap;
}
.sources {
  display: inline-flex;
  gap: 2px;
  padding: 3px;
  border-radius: 999px;
  background: var(--pill-bg);
  border: 1px solid var(--hairline);
}
.src {
  height: 30px;
  padding: 0 16px;
  border: 0;
  border-radius: 999px;
  background: transparent;
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text-2);
  transition:
    background 0.18s ease,
    color 0.18s ease,
    box-shadow 0.18s ease;
}
.src:hover {
  color: var(--text);
  background: var(--hover);
}
.src.on {
  background: var(--accent);
  color: #fff;
  box-shadow: 0 4px 12px color-mix(in srgb, var(--accent) 30%, transparent);
}
.src:focus-visible {
  outline: 2px solid color-mix(in srgb, var(--accent) 45%, transparent);
  outline-offset: 2px;
}
.ov-search {
  margin-top: 12px;
  display: flex;
  align-items: center;
  gap: 10px;
  height: 48px;
  padding: 0 6px 0 14px;
  border-radius: 14px;
  border: 1px solid var(--hairline);
  background: color-mix(in srgb, var(--bg) 72%, transparent);
  transition:
    border-color 0.2s ease,
    box-shadow 0.2s ease,
    background 0.2s ease;
}
.ov-search:focus-within {
  border-color: color-mix(in srgb, var(--accent) 55%, var(--hairline));
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 16%, transparent);
  background: color-mix(in srgb, var(--bg) 88%, transparent);
}
.s-ico {
  flex: none;
  width: 18px;
  height: 18px;
  color: var(--text-3);
}
.ov-search input {
  flex: 1;
  min-width: 0;
  height: 100%;
  border: 0;
  outline: none;
  background: transparent;
  color: var(--text);
  font-size: 14.5px;
}
.ov-search input::placeholder {
  color: var(--text-3);
}
.clear {
  flex: none;
  width: 28px;
  height: 28px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-3);
}
.clear:hover {
  background: var(--hover);
  color: var(--text);
}
.clear svg {
  width: 13px;
  height: 13px;
}
.go {
  flex: none;
  height: 36px;
  padding: 0 20px;
  border: 0;
  border-radius: 999px;
  background: var(--accent);
  color: #fff;
  font-size: 13.5px;
  font-weight: 600;
  transition: background 0.18s ease;
}
.go:disabled {
  opacity: 0.45;
  cursor: default;
}
.go:hover:not(:disabled) {
  background: var(--accent-hover);
}

@media (max-width: 760px) {
  .panel-hint {
    display: none;
  }
  .search-panel {
    padding: 10px;
  }
  .src {
    padding: 0 12px;
  }
}
</style>
