<script setup lang="ts">
import { lib, reloadLibrary, addFolder, openImportDir } from "../stores/library";
</script>

<template>
  <!-- 曲库加载守卫:未加载完/读取失败时占位,成功才渲染插槽内容 -->
  <div v-if="!lib.loaded" class="empty">
    <h2>正在加载曲库…</h2>
  </div>
  <div v-else-if="lib.loadError" class="empty">
    <h2>曲库读取失败</h2>
    <p class="err">{{ lib.loadError }}</p>
    <button class="primary-pill" @click="reloadLibrary">重试</button>
  </div>
  <!-- 空曲库引导:从「最近添加」页移除后由守卫统一承接(每个视图都包在 LibGate 里) -->
  <div v-else-if="lib.folders.length === 0" class="empty">
    <svg class="icon" width="64" height="64" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round">
      <path d="M9.2 17.3V7l9-1.9v10.2" />
      <circle cx="6.9" cy="17.4" r="2.3" />
      <circle cx="15.9" cy="15.4" r="2.3" />
    </svg>
    <h2>欢迎使用 TauriMusic</h2>
    <p>添加一个音乐文件夹,自动识别封面、歌词与专辑信息</p>
    <button class="primary-pill" style="margin-top: 8px" @click="addFolder">添加音乐文件夹</button>
    <div class="alt">
      <button class="link" @click="openImportDir">打开导入文件夹</button>
      <span class="dot">·</span>
      <button class="link" @click="addFolder">添加其他文件夹</button>
    </div>
    <p class="dir" :title="lib.importDir">{{ lib.importDir }}</p>
    <p class="formats">支持 MP3 / FLAC / M4A / OGG / OPUS / WAV · 歌词支持内嵌或同名 .lrc · 封面自动识别</p>
  </div>
  <slot v-else />
</template>

<style scoped>
.err {
  color: var(--text-2);
  font-size: 12px;
  max-width: 480px;
  word-break: break-all;
}
.alt {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12.5px;
}
.dot {
  color: var(--text-3);
}
.link {
  color: var(--accent);
  font-size: 12.5px;
}
.link:hover {
  text-decoration: underline;
}
.empty {
  height: 70%;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  color: var(--text-2);
}
.empty .icon {
  color: var(--text-3);
  margin-bottom: 6px;
}
.empty h2 {
  font-size: 17px;
  color: var(--text);
}
.empty p {
  font-size: 13.5px;
}
.empty .dir {
  font-size: 11.5px;
  color: var(--text-3);
  max-width: 70%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.empty .formats {
  font-size: 11.5px;
  color: var(--text-3);
}
</style>
