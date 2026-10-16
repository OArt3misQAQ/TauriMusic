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
const showLetter = ref(false);
let copiedTimer = 0;
let usageTimer = 0;

/* ═══════════════════════════════════════════════════════════════
   作者感谢信 —— 内容就在这里填写,界面上不用再改任何东西

   · LETTER_PARAGRAPHS:正文段落数组,每个字符串是一段,按顺序显示。
     例:LETTER_PARAGRAPHS = ["第一段……", "第二段……"]
   · LETTER_SIGN:落款一行(署名/日期),留空则整个落款区不显示。
     例:LETTER_SIGN = "O_Art3mis · 2026 年秋"

   两处都留空时,信纸只显示几条淡淡的待写横线,不影响外观。 */
const LETTER_PARAGRAPHS: string[] = [
  // ← 在这行下面按 "……", 的格式一段一条地写
];
const LETTER_SIGN = ""; // ← 落款写在这里
/* ═══════════════════════════════════════════════════════════════ */

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

onUnmounted(() => {
  window.clearInterval(usageTimer);
  // copiedTimer 不清的话,弹窗关闭后 2 秒仍会对已卸载组件的 ref 空写一次
  window.clearTimeout(copiedTimer);
});

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
    <!-- 感谢信视图 -->
    <template v-if="showLetter">
      <header class="head">
        <button class="back" title="返回关于" @click="showLetter = false">
          <span class="arr">‹</span>返回
        </button>
        <h2 class="ltitle">作者的感谢信</h2>
        <span class="head-pad" aria-hidden="true"></span>
      </header>

      <div class="letter">
        <p v-for="(p, i) in LETTER_PARAGRAPHS" :key="i" class="para">{{ p }}</p>
        <!-- 正文留空时显示待写横线(信纸格),不显示任何提示文字 -->
        <div v-if="!LETTER_PARAGRAPHS.length" class="lines" aria-hidden="true">
          <i></i><i></i><i></i><i></i>
        </div>
        <p v-if="LETTER_SIGN" class="sign">{{ LETTER_SIGN }}</p>
      </div>
    </template>

    <!-- 关于视图 -->
    <template v-else>
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

      <button class="letter-entry" @click="showLetter = true">
        <span>作者的感谢信</span>
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round">
          <path d="m9 5 7 7-7 7" />
        </svg>
      </button>

      <p v-if="usage" class="res">
        内存 {{ usage.memory_mb.toFixed(1) }} MB · CPU {{ usage.cpu.toFixed(1) }}%
      </p>

      <p class="tech">Tauri 2 · Vue 3 · Rust · lofty</p>
    </template>
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

/* 感谢信视图的头部:返回按钮与标题(与专辑详情页的红色 ‹ 返回同一语言) */
.back {
  display: flex;
  align-items: center;
  gap: 2px;
  font-size: 13.5px;
  font-weight: 600;
  color: var(--accent);
  border-radius: 8px;
  padding: 4px 8px 4px 4px;
  transition: background 0.2s ease;
}
.back:hover {
  background: var(--hover);
}
.back .arr {
  font-size: 17px;
  line-height: 1;
  transform: translateY(-1px);
}
.ltitle {
  flex: 1;
  text-align: center;
  margin-right: 52px;
}
.head-pad {
  width: 52px;
  flex: none;
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

/* 感谢信入口:与作者信息行同构的整行按钮 */
.letter-entry {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 10px;
  border: 1px solid var(--hairline);
  border-radius: 12px;
  padding: 12px 14px;
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
  transition: background 0.2s ease;
}
.letter-entry:hover {
  background: var(--hover);
}
.letter-entry svg {
  width: 14px;
  height: 14px;
  margin-left: auto;
  color: var(--text-3);
}

/* 信纸:衬线字体配行距,是"信"而不是 UI 文本;
   颜色全走令牌,深浅色与两种皮肤自动适配 */
.letter {
  border: 1px solid var(--hairline);
  border-radius: 12px;
  background: var(--bg-2);
  padding: 24px 26px;
  font-family: Georgia, "Times New Roman", "Noto Serif SC", "Source Han Serif SC", "Songti SC", SimSun, serif;
}
.letter .para {
  font-size: 13.5px;
  line-height: 2;
  color: var(--text);
  text-align: justify;
  margin: 0 0 14px;
}
.letter .para:last-of-type {
  margin-bottom: 0;
}
/* 正文待写的空信纸:几条淡横线撑起信纸的"格" */
.letter .lines i {
  display: block;
  height: 1px;
  background: var(--hairline);
  margin: 30px 0;
}
.letter .sign {
  margin-top: 18px;
  padding-top: 12px;
  border-top: 1px solid var(--hairline);
  text-align: right;
  font-size: 13px;
  color: var(--text-2);
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
