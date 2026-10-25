<script setup lang="ts">
import { computed } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import { player, current, toggle, next, coverPathOf } from "../stores/player";
import { ui } from "../stores/ui";

const cover = computed(() => {
  // 迷你条只有 42px:优先 256px 缩略图,别把 512px 大图拉进内存
  const t = current.value;
  const p = t?.cover ?? coverPathOf(t);
  return p ? convertFileSrc(p) : null;
});
const progress = computed(() =>
  player.duration > 0 ? Math.min(100, (player.position / player.duration) * 100) : 0,
);

function openPlayer(): void {
  if (current.value) ui.nowPlayingOpen = true;
}
</script>

<template>
  <div
    v-if="current"
    class="mini"
    :style="{ '--mini-cover': cover ? `url('${cover}')` : 'none' }"
    role="button"
    tabindex="0"
    title="打开播放页"
    @click="openPlayer"
    @keydown.enter="openPlayer"
    @keydown.space.prevent="openPlayer"
  >
    <i class="mini-wash" aria-hidden="true"></i>
    <span class="mini-line" :style="{ width: progress + '%' }"></span>
    <span class="mini-cover">
      <img v-if="cover" :src="cover" alt="" />
      <span v-else class="ph">♪</span>
    </span>
    <span class="mini-meta">
      <span class="mini-title">{{ current.title }}</span>
      <span class="mini-artist">{{ current.artist }}</span>
    </span>
    <button
      class="mini-btn play"
      :title="player.playing ? '暂停' : '播放'"
      :aria-label="player.playing ? '暂停' : '播放'"
      @click.stop="toggle()"
    >
      <svg v-if="!player.playing" viewBox="0 0 24 24"><path d="M8.2 5.5v13L19 12z" fill="currentColor" /></svg>
      <svg v-else viewBox="0 0 24 24" fill="currentColor">
        <rect x="7" y="5.2" width="3.6" height="13.6" rx="1.2" />
        <rect x="13.4" y="5.2" width="3.6" height="13.6" rx="1.2" />
      </svg>
    </button>
    <button class="mini-btn" title="下一曲" aria-label="下一曲" @click.stop="next()">
      <svg viewBox="0 0 24 24" fill="currentColor">
        <path d="M12.6 6.4v11.2L20.4 12z" />
        <path d="M4 6.4v11.2L11.8 12z" />
      </svg>
    </button>
  </div>
</template>

<style scoped>
.mini {
  position: relative;
  flex: none;
  display: flex;
  align-items: center;
  gap: 10px;
  margin: 6px 10px 8px;
  padding: 9px 10px;
  min-height: 58px;
  border-radius: 14px;
  border: 1px solid var(--hairline);
  background: var(--bar-bg);
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.16);
  backdrop-filter: blur(24px) saturate(1.6);
  -webkit-backdrop-filter: blur(24px) saturate(1.6);
  overflow: hidden;
  cursor: pointer;
}
.mini-wash {
  position: absolute;
  inset: -20px;
  background-image: var(--mini-cover, none);
  background-size: cover;
  background-position: center;
  filter: blur(24px) saturate(1.5);
  opacity: 0.32;
  pointer-events: none;
}
.mini-line {
  position: absolute;
  z-index: 2;
  top: 0;
  left: 0;
  height: 2px;
  background: var(--accent);
  border-radius: 0 2px 2px 0;
  transition: width 0.3s linear;
}
.mini-cover {
  position: relative;
  z-index: 1;
  width: 42px;
  height: 42px;
  flex: none;
  border-radius: 8px;
  overflow: hidden;
  background: var(--bg-3);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-3);
  font-size: 18px;
}
.mini-cover img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}
.mini-meta {
  position: relative;
  z-index: 1;
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.mini-title,
.mini-artist {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.mini-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
}
.mini-artist {
  font-size: 11px;
  color: var(--text-2);
}
.mini-btn {
  position: relative;
  z-index: 1;
  width: 38px;
  height: 38px;
  flex: none;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-2);
  transition: background 0.15s ease, transform 0.2s var(--ease-spring);
}
.mini-btn svg {
  width: 19px;
  height: 19px;
}
.mini-btn.play svg {
  width: 23px;
  height: 23px;
  color: var(--text);
}
.mini-btn:active {
  background: var(--hover);
  transform: scale(0.9);
  transition-duration: 0.08s;
}
.mini:active {
  background: color-mix(in srgb, var(--bar-bg) 86%, var(--active));
}
</style>
