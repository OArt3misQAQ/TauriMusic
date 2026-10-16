<script setup lang="ts">
import { computed } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import Modal from "./Modal.vue";
import type { PackagePreview, PreviewTrack } from "../api";
import { closeImportPreview, confirmPackageImport, importPreview } from "../stores/library";

/** 音乐包导入预览弹窗:双击/拖入 .tmc/.tmca/.tmcl 时先展示包内曲目,
 *  确认后才正式导入。三段状态机 preview → importing → done(见 stores/library.ts) */

const KIND_LABEL: Record<string, string> = {
  tmc: "单曲包",
  tmca: "专辑包",
  tmcl: "歌单包",
};

const totalTracks = computed(() =>
  importPreview.packages.reduce((n, p) => n + p.tracks.length, 0),
);

/** 勾选包内的曲目数(按钮按"首"计,不能报包数) */
const checkedCount = computed(() =>
  importPreview.packages.reduce((n, p) => n + (p.checked ? p.tracks.length : 0), 0),
);

const allChecked = computed(
  () => importPreview.packages.length > 0 && checkedCount.value === importPreview.packages.length,
);

/** 主按钮文案:全勾 = 全部导入;勾了部分 = 导入所选 */
const importLabel = computed(() =>
  allChecked.value
    ? `全部导入 ${checkedCount.value} 首`
    : `导入所选 ${checkedCount.value} 首`,
);

const phaseHint = computed(() => {
  if (importPreview.phase === "preview") return "导入不会移动原文件,音乐包里的音频会复制进曲库目录";
  if (importPreview.phase === "importing")
    return `正在导入 ${importPreview.current} / ${importPreview.packages.length} 个音乐包`;
  return importPreview.summary || "完成";
});

function kindLabel(kind: string): string {
  return KIND_LABEL[kind] ?? "音乐包";
}

function fmt(s: number): string {
  const v = Number.isFinite(s) && s > 0 ? Math.floor(s) : 0;
  return `${Math.floor(v / 60)}:${String(v % 60).padStart(2, "0")}`;
}

function coverSrc(pkg: PackagePreview, t: PreviewTrack): string | undefined {
  return t.cover ? convertFileSrc(`${pkg.root}/${t.cover}`) : undefined;
}
</script>

<template>
  <Modal @close="closeImportPreview">
    <header class="head">
      <div class="titles">
        <h2>导入音乐包</h2>
        <p class="sub">确认后加入曲库</p>
      </div>
      <span class="count-chip">{{ totalTracks }} 首</span>
    </header>

    <!-- 预览:逐包列出曲目(封面/歌名/艺人/时长),行入场按序错峰 -->
    <div v-if="importPreview.phase === 'preview'" class="body">
      <section
        v-for="pkg in importPreview.packages"
        :key="pkg.path"
        class="pack"
        :class="{ skipped: !pkg.checked }"
      >
        <div class="pack-head">
          <input
            v-model="pkg.checked"
            class="pick"
            type="checkbox"
            :title="pkg.checked ? '取消勾选' : '加入导入'"
          />
          <img
            v-if="pkg.tracks[0]?.cover"
            class="hero"
            :src="coverSrc(pkg, pkg.tracks[0])"
            alt=""
          />
          <div v-else class="hero ph">♪</div>
          <div class="pack-meta">
            <span class="badge">{{ kindLabel(pkg.kind) }}</span>
            <h3 class="pack-name" :title="pkg.name">{{ pkg.name }}</h3>
            <span class="dim">{{ pkg.tracks.length }} 首</span>
          </div>
        </div>
        <ul class="rows">
          <li
            v-for="(t, i) in pkg.tracks"
            :key="i"
            class="row"
            :style="{ animationDelay: `${Math.min(i, 12) * 36}ms` }"
          >
            <span class="idx">{{ i + 1 }}</span>
            <img v-if="t.cover" class="thumb" :src="coverSrc(pkg, t)" alt="" />
            <span v-else class="thumb ph">♪</span>
            <span class="names">
              <span class="t" :title="t.title">{{ t.title }}</span>
              <span class="a" :title="t.artist">{{ t.artist }}<template v-if="t.album"> — {{ t.album }}</template></span>
            </span>
            <span class="d">{{ fmt(t.duration) }}</span>
          </li>
        </ul>
      </section>
      <p v-if="importPreview.error" class="err">{{ importPreview.error }}</p>
      <p class="empty" v-if="!importPreview.packages.length && !importPreview.error">
        正在解析音乐包…
      </p>
    </div>

    <!-- 导入中/完成:本次导入的歌曲清单 + 逐包结果 -->
    <div v-else class="body">
      <div class="stage">
        <span v-if="importPreview.phase === 'importing'" class="spinner" aria-hidden="true"></span>
        <span v-else class="done-mark" aria-hidden="true">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">
            <path d="m5 12.5 4.5 4.5L19 7.5" />
          </svg>
        </span>
        <p class="stage-text">
          {{ importPreview.phase === "importing" ? "正在写入曲库…" : "导入完成" }}
        </p>
      </div>

      <!-- 本次实际新导入的歌曲(重扫后与导入前快照做差) -->
      <template v-if="importPreview.phase === 'done'">
        <div v-if="importPreview.newTracks.length" class="new-wrap">
          <p class="new-head">本次导入的歌曲</p>
          <ul class="rows">
            <li
              v-for="(t, i) in importPreview.newTracks"
              :key="i"
              class="row"
              :style="{ animationDelay: `${Math.min(i, 12) * 36}ms` }"
            >
              <span class="idx">{{ i + 1 }}</span>
              <img v-if="t.cover" class="thumb" :src="convertFileSrc(t.cover)" alt="" />
              <span v-else class="thumb ph">♪</span>
              <span class="names">
                <span class="t" :title="t.title">{{ t.title }}</span>
                <span class="a" :title="t.artist">{{ t.artist }}</span>
              </span>
            </li>
          </ul>
        </div>
        <p v-else class="empty">没有新增曲目:音乐包内容与曲库完全相同,已全部跳过</p>
      </template>

      <ul v-if="importPreview.results.length" class="results">
        <li v-for="(r, i) in importPreview.results" :key="i">{{ r }}</li>
      </ul>
      <p v-if="importPreview.error" class="err">{{ importPreview.error }}</p>
    </div>

    <footer class="foot">
      <span class="hint">{{ phaseHint }}</span>
      <div class="btns">
        <button
          v-if="importPreview.phase === 'preview'"
          class="btn ghost"
          @click="closeImportPreview"
        >
          取消
        </button>
        <button
          v-if="importPreview.phase === 'preview'"
          class="btn primary"
          :disabled="!checkedCount"
          @click="confirmPackageImport"
        >
          {{ importLabel }}
        </button>
        <button v-else-if="importPreview.phase === 'done'" class="btn primary" @click="closeImportPreview">
          完成
        </button>
      </div>
    </footer>
  </Modal>
</template>

<style scoped>
.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 14px;
}
.titles h2 {
  font-size: 18px;
  font-weight: 700;
}
.sub {
  font-size: 12px;
  color: var(--text-2);
  margin-top: 2px;
}
.count-chip {
  font-size: 11.5px;
  font-weight: 700;
  color: var(--accent);
  background: color-mix(in srgb, var(--accent) 12%, transparent);
  border: 1px solid color-mix(in srgb, var(--accent) 24%, transparent);
  border-radius: 999px;
  padding: 3px 12px;
  font-variant-numeric: tabular-nums;
}

.body {
  max-height: 46vh;
  overflow-y: auto;
  scrollbar-width: none;
}
.body::-webkit-scrollbar {
  display: none;
}

.pack {
  margin-bottom: 18px;
  transition: opacity 0.25s ease;
}
.pack.skipped {
  opacity: 0.45;
}
.pack-head {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 8px;
}
/* 勾选框决定该包是否参与导入;未勾选的包整段降透明 */
.pick {
  width: 15px;
  height: 15px;
  accent-color: var(--accent);
  cursor: pointer;
  flex: none;
}
.hero {
  width: 52px;
  height: 52px;
  border-radius: 8px;
  object-fit: cover;
  box-shadow: 0 3px 10px rgba(0, 0, 0, 0.22);
}
.hero.ph {
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 20px;
  color: var(--text-3);
  background: var(--pill-bg);
}
.pack-meta {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.badge {
  align-self: flex-start;
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.06em;
  color: var(--text-2);
  border: 1px solid var(--hairline);
  border-radius: 4px;
  padding: 1px 6px;
}
.pack-name {
  font-size: 14px;
  font-weight: 700;
  color: var(--text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.dim {
  font-size: 11px;
  color: var(--text-3);
}

.rows {
  border: 1px solid var(--hairline);
  border-radius: 10px;
  overflow: hidden;
}
.row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 7px 12px;
  animation: row-in 0.34s var(--ease-out-soft) both;
}
.row + .row {
  border-top: 1px solid var(--hairline);
}
.row:hover {
  background: var(--hover);
}
@keyframes row-in {
  from {
    opacity: 0;
    transform: translateY(7px);
  }
  to {
    opacity: 1;
    transform: none;
  }
}
.idx {
  width: 16px;
  flex: none;
  text-align: center;
  font-size: 10.5px;
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
}
.thumb {
  width: 32px;
  height: 32px;
  border-radius: 5px;
  object-fit: cover;
  flex: none;
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.18);
}
.thumb.ph {
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 13px;
  color: var(--text-3);
  background: var(--pill-bg);
}
.names {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}
.t {
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.a {
  font-size: 10.5px;
  color: var(--text-2);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.d {
  flex: none;
  font-size: 10.5px;
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
}

.err {
  font-size: 11.5px;
  color: var(--danger);
  margin-top: 10px;
  line-height: 1.6;
}
.empty {
  padding: 24px 0;
  text-align: center;
  font-size: 12px;
  color: var(--text-3);
}

/* 导入中/完成 */
.stage {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 10px;
  padding: 18px 0 14px;
}
.spinner {
  width: 26px;
  height: 26px;
  border-radius: 50%;
  border: 3px solid var(--pill-bg);
  border-top-color: var(--accent);
  animation: ip-spin 0.8s linear infinite;
}
@keyframes ip-spin {
  to {
    transform: rotate(360deg);
  }
}
.done-mark {
  width: 40px;
  height: 40px;
  border-radius: 50%;
  background: var(--accent);
  color: #fff;
  display: flex;
  align-items: center;
  justify-content: center;
  box-shadow: 0 4px 14px color-mix(in srgb, var(--accent) 45%, transparent);
}
.done-mark svg {
  width: 20px;
  height: 20px;
}
.stage-text {
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
}
.new-wrap {
  margin-bottom: 12px;
}
.new-head {
  font-size: 11px;
  font-weight: 700;
  letter-spacing: 0.06em;
  color: var(--text-3);
  margin-bottom: 6px;
}
.results {
  border: 1px solid var(--hairline);
  border-radius: 10px;
  overflow: hidden;
}
.results li {
  padding: 8px 12px;
  font-size: 12px;
  color: var(--text-2);
}
.results li + li {
  border-top: 1px solid var(--hairline);
}

.foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-top: 16px;
}
.hint {
  font-size: 11px;
  color: var(--text-3);
  min-width: 0;
}
.btns {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: none;
}
.btn {
  font-size: 12.5px;
  font-weight: 600;
  border-radius: 8px;
  padding: 8px 18px;
  transition: background 0.2s ease, transform 0.25s var(--ease-spring);
}
.btn:active {
  transform: scale(0.96);
}
.btn.ghost {
  color: var(--text);
  background: var(--pill-bg);
}
.btn.ghost:hover {
  background: var(--hover);
}
.btn.primary {
  background: var(--accent);
  color: #fff;
  box-shadow: 0 3px 10px color-mix(in srgb, var(--accent) 40%, transparent);
}
.btn.primary:hover:not(:disabled) {
  background: var(--accent-hover);
}
.btn.primary:disabled {
  opacity: 0.5;
  cursor: default;
}
</style>
