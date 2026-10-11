<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { getVersion } from "@tauri-apps/api/app";
import Modal from "./Modal.vue";
import { AUTHOR_MAIL } from "../constants";
import { ui } from "../stores/ui";
import { api, type ResourceUsage } from "../api";


const version = ref("0.1.0");
const copied = ref(false);
const usage = ref<ResourceUsage | null>(null);
let copiedTimer = 0;
let usageTimer = 0;

async function refreshUsage(): Promise<void> {
  try {
    usage.value = await api.getResourceUsage();
  } catch {
    /* 静默失败,不影响关于页 */
  }
}

onMounted(async () => {
  try {
    version.value = await getVersion();
  } catch {
    /* 保持默认版本号 */
  }
  void refreshUsage();
  usageTimer = window.setInterval(refreshUsage, 2000);
});

onUnmounted(() => window.clearInterval(usageTimer));

async function copyMail(): Promise<void> {
  try {
    await navigator.clipboard.writeText(AUTHOR_MAIL);
    copied.value = true;
    window.clearTimeout(copiedTimer);
    copiedTimer = window.setTimeout(() => (copied.value = false), 2000);
  } catch {
    window.location.href = `mailto:${AUTHOR_MAIL}`;
  }
}
</script>

<template>
  <Modal @close="ui.aboutOpen = false">
    <header class="head">
      <h2>关于</h2>
      <button class="close" title="关闭" @click="ui.aboutOpen = false">
        <svg viewBox="0 0 24 24"><path d="m6 6 12 12M18 6 6 18" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /></svg>
      </button>
    </header>

    <div class="hero">
      <span class="logo">
        <svg viewBox="0 0 24 24">
          <!-- 迷你黑胶:与应用图标「墨夜胶片」同一几何语言,中孔透出陶土底色 -->
          <circle cx="12" cy="12" r="8.6" fill="none" stroke="#fff" stroke-width="2" />
          <circle cx="12" cy="12" r="3.4" fill="#fff" />
          <circle cx="12" cy="12" r="1.2" style="fill: var(--accent)" />
        </svg>
      </span>
      <div>
        <div class="name">TauriMusic</div>
        <div class="ver">版本 {{ version }}</div>
      </div>
    </div>

    <p class="desc">本地音乐播放器,自动识别封面、歌词与专辑信息。</p>

    <div class="author">
      <span class="label">作者</span>
      <a class="mail" :href="`mailto:${AUTHOR_MAIL}`">{{ AUTHOR_MAIL }}</a>
      <button class="mini" @click="copyMail">{{ copied ? "已复制" : "复制" }}</button>
    </div>

    <p v-if="usage" class="res">
      内存 {{ usage.memory_mb.toFixed(1) }} MB · CPU {{ usage.cpu.toFixed(1) }}%
    </p>

    <p class="tech">Tauri 2 · Vue 3 · Rust · lofty</p>
  </Modal>
</template>

<style scoped>
.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 16px;
}
.head h2 {
  font-size: 18px;
  font-weight: 700;
}
.close {
  width: 28px;
  height: 28px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-2);
  transition: background 0.2s ease, color 0.2s ease;
}
.close:hover {
  background: var(--hover);
  color: var(--text);
}
.close svg {
  width: 15px;
  height: 15px;
}

.hero {
  display: flex;
  align-items: center;
  gap: 14px;
}
.logo {
  width: 52px;
  height: 52px;
  border-radius: 13px;
  background: var(--accent);
  display: flex;
  align-items: center;
  justify-content: center;
  box-shadow: 0 4px 14px rgba(0, 0, 0, 0.25);
  flex: none;
}
.logo svg {
  width: 34px;
  height: 34px;
}
.name {
  font-size: 19px;
  font-weight: 700;
  color: var(--text);
}
.ver {
  font-size: 12px;
  color: var(--text-2);
  margin-top: 2px;
}

.desc {
  font-size: 13px;
  color: var(--text-2);
  margin: 14px 0 16px;
  line-height: 1.7;
}

.author {
  display: flex;
  align-items: center;
  gap: 10px;
  border: 1px solid var(--hairline);
  border-radius: 12px;
  padding: 12px 14px;
}
.label {
  font-size: 12.5px;
  color: var(--text-2);
  flex: none;
}
.mail {
  font-size: 13px;
  font-weight: 600;
  color: var(--accent);
  text-decoration: none;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.mail:hover {
  text-decoration: underline;
}
.mini {
  flex: none;
  margin-left: auto;
  font-size: 12px;
  font-weight: 600;
  color: var(--text);
  background: var(--pill-bg);
  border-radius: 999px;
  padding: 5px 14px;
  transition: background 0.2s ease;
}
.mini:hover {
  background: var(--hover);
}

.res {
  font-size: 11.5px;
  color: var(--text-3);
  margin-top: 16px;
  text-align: center;
  font-variant-numeric: tabular-nums;
}

.tech {
  font-size: 11.5px;
  color: var(--text-3);
  margin-top: 6px;
  text-align: center;
}
</style>
