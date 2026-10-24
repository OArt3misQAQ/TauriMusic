<script setup lang="ts">
import { computed } from "vue";
import Modal from "./Modal.vue";
import { closeExport, confirmExport, exportState } from "../stores/library";

/** 打包阶段的主进度:TMC 多包按包裹数推进,TMCL 单大包按内部资源数推进 */
const pct = computed(() => {
  if (exportState.phase !== "packing") return 100;
  if (exportState.assetTotal > 0) {
    return Math.min(100, (exportState.assetDone / exportState.assetTotal) * 100);
  }
  return exportState.total > 0 ? (exportState.done / exportState.total) * 100 : 0;
});

const packingHint = computed(() => {
  if (exportState.assetTotal > 0) {
    return `正在打包音乐文件 ${exportState.assetDone}/${exportState.assetTotal}`;
  }
  if (exportState.total > 1) {
    return `正在准备第 ${Math.min(exportState.done + 1, exportState.total)}/${exportState.total} 首`;
  }
  return "正在准备音乐包";
});
</script>

<template>
  <Modal @close="closeExport">
    <header class="head">
      <h2>导出音乐包</h2>
      <button class="close" title="关闭" @click="closeExport">
        <svg viewBox="0 0 24 24">
          <path d="m6 6 12 12M18 6 6 18" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
        </svg>
      </button>
    </header>

    <div v-if="exportState.label" class="label">{{ exportState.label }}</div>

    <!-- 打包中:进度条 + 当前歌名 -->
    <template v-if="exportState.phase === 'packing'">
      <div v-if="exportState.err" class="err">{{ exportState.err }}</div>
      <template v-else>
        <div class="bar">
          <div class="fill" :style="{ width: pct + '%' }"></div>
        </div>
        <div class="now">
          <span class="spinner" aria-hidden="true"></span>
          <span class="txt">
            <span class="name" :title="exportState.current">{{ exportState.current || "正在准备…" }}</span>
            <span class="hint">{{ packingHint }}</span>
          </span>
        </div>
        <p class="note">准备在后台进行,完成后点「导出」选择保存位置,文件会一次性完整写入。</p>
      </template>
    </template>

    <!-- 就绪:等待用户点「导出」 -->
    <template v-else-if="exportState.phase === 'ready'">
      <div class="row">
        <svg class="ok" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">
          <path d="m5 12.5 4.5 4.5L19 8" />
        </svg>
        <div class="row-txt">
          <span class="name">{{ exportState.files.length }} 个音乐包已准备好</span>
          <span class="hint">点「导出」选择保存位置,文件会立即完整写入。</span>
        </div>
      </div>
      <div v-if="exportState.packFailed" class="warn">
        {{ exportState.packFailed }} 首无法打包(文件缺失或被占用),已跳过
      </div>
      <p class="note">
        音乐包内容由你的本地曲库生成,请仅限个人使用;分享所产生的法律风险由使用者自行承担,与 TauriMusic 无关。
      </p>
    </template>

    <!-- 落位中:把完整的音乐包写入所选位置 -->
    <template v-else-if="exportState.phase === 'placing'">
      <div class="bar">
        <div class="fill indeterminate"></div>
      </div>
      <div class="now">
        <span class="spinner" aria-hidden="true"></span>
        <span class="txt"><span class="name">正在保存到所选位置…</span></span>
      </div>
    </template>

    <!-- 完成 -->
    <template v-else>
      <div class="row">
        <svg class="ok" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">
          <path d="m5 12.5 4.5 4.5L19 8" />
        </svg>
        <div class="row-txt">
          <span class="name">已导出 {{ exportState.placed }} 个音乐包</span>
          <span v-if="exportState.failed" class="hint">{{ exportState.failed }} 个保存失败(文件被占用)</span>
        </div>
      </div>
      <div v-if="exportState.err && !exportState.placed" class="err">{{ exportState.err }}</div>
    </template>

    <footer class="foot">
      <template v-if="exportState.phase === 'packing'">
        <button class="ghost" @click="closeExport">{{ exportState.err ? "关闭" : "取消" }}</button>
      </template>
      <template v-else-if="exportState.phase === 'ready'">
        <button class="ghost" @click="closeExport">取消</button>
        <button class="pill pill--accent primary-pill" @click="confirmExport">导出…</button>
      </template>
      <template v-else-if="exportState.phase === 'done'">
        <button class="pill pill--accent primary-pill" @click="closeExport">完成</button>
      </template>
    </footer>
  </Modal>
</template>

<style scoped>
.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 6px;
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

.label {
  font-size: 12.5px;
  font-weight: 600;
  color: var(--accent);
  margin-bottom: 12px;
}

.bar {
  height: 8px;
  border-radius: 999px;
  background: var(--pill-bg);
  overflow: hidden;
  margin: 10px 0 14px;
}
.fill {
  height: 100%;
  border-radius: inherit;
  background: var(--accent);
  transition: width 0.35s var(--ease-out-soft);
}
.fill.indeterminate {
  width: 36%;
  animation: slide 1.1s ease-in-out infinite;
}
@keyframes slide {
  0% {
    transform: translateX(-110%);
  }
  100% {
    transform: translateX(320%);
  }
}

.now {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
}
.spinner {
  width: 16px;
  height: 16px;
  flex: none;
  border-radius: 50%;
  border: 2px solid var(--pill-bg);
  border-top-color: var(--accent);
  animation: spin 0.9s linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
.row-txt,
.txt {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.name {
  font-size: 14px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.hint {
  font-size: 12.5px;
  color: var(--text-2);
}

.row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 8px 0 4px;
}
.ok {
  width: 34px;
  height: 34px;
  flex: none;
  padding: 7px;
  box-sizing: border-box;
  border-radius: 50%;
  color: #fff;
  background: var(--accent);
}

.note {
  font-size: 12px;
  color: var(--text-3);
  margin-top: 12px;
  line-height: 1.5;
}
.warn {
  margin-top: 10px;
  font-size: 12.5px;
  color: var(--danger);
  background: var(--danger-bg);
  border-radius: 8px;
  padding: 8px 12px;
}
.err {
  margin-top: 8px;
  font-size: 13px;
  color: var(--danger);
  background: var(--danger-bg);
  border-radius: 8px;
  padding: 10px 12px;
  word-break: break-all;
}

.foot {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  margin-top: 18px;
}
.ghost {
  padding: 8px 18px;
  border-radius: 999px;
  font-size: 13.5px;
  font-weight: 600;
  color: var(--text-2);
  transition: background 0.2s ease, color 0.2s ease;
}
.ghost:hover {
  background: var(--hover);
  color: var(--text);
}
</style>
