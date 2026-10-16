<script setup lang="ts">
import { nextTick, ref, watch } from "vue";
import Modal from "./Modal.vue";
import { nameDialog, submitPlaylistName } from "../stores/playlists";

const value = ref("");
const inputEl = ref<HTMLInputElement | null>(null);

watch(
  () => nameDialog.open,
  async (open) => {
    if (!open) return;
    value.value = nameDialog.initial;
    await nextTick();
    inputEl.value?.focus();
    inputEl.value?.select();
  },
  // 组件本身 v-if 挂载:创建时 open 已经是 true,之后不再变化——
  // 没有 immediate 的话重命名的旧名永远带不进来,输入框空白且无焦点
  { immediate: true },
);

function confirm(): void {
  void submitPlaylistName(value.value);
}

function close(): void {
  nameDialog.open = false;
}
</script>

<template>
  <Modal v-if="nameDialog.open" @close="close">
    <header class="head">
      <h2>{{ nameDialog.title }}</h2>
      <button class="close" title="关闭" @click="close">
        <svg viewBox="0 0 24 24"><path d="m6 6 12 12M18 6 6 18" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /></svg>
      </button>
    </header>

    <input
      ref="inputEl"
      v-model="value"
      class="name-input"
      type="text"
      maxlength="60"
      placeholder="播放列表名称"
      spellcheck="false"
      @keydown.enter.prevent="confirm"
      @keydown.esc.prevent="close"
    />

    <div class="actions">
      <button class="mini ghost" @click="close">取消</button>
      <button class="mini primary" :disabled="!value.trim()" @click="confirm">确定</button>
    </div>
  </Modal>
</template>

<style scoped>
.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 14px;
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

.name-input {
  width: 100%;
  border: 1px solid var(--hairline);
  background: var(--pill-bg);
  border-radius: 10px;
  padding: 10px 12px;
  font-size: 14px;
  color: var(--text);
  outline: none;
  transition: border-color 0.2s ease, box-shadow 0.2s ease;
}
.name-input:focus {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 15%, transparent);
}

.actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 16px;
}
.mini {
  font-size: 13px;
  font-weight: 600;
  border-radius: 999px;
  padding: 7px 18px;
  transition: background 0.2s ease, transform 0.35s var(--ease-spring);
}
.mini.ghost {
  color: var(--text);
  background: var(--pill-bg);
}
.mini.ghost:hover {
  background: var(--hover);
}
.mini.primary {
  color: #fff;
  background: var(--accent);
}
.mini.primary:hover:not(:disabled) {
  background: var(--accent-hover);
}
.mini.primary:disabled {
  opacity: 0.5;
  cursor: default;
}
</style>
