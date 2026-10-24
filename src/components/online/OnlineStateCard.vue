<script setup lang="ts">
/** 结果区的四态卡:错误/加载中/空结果/初始引导,图标与文案随 kind 切换,容器样式统一 */
defineProps<{
  kind: "err" | "loading" | "empty" | "idle";
  title: string;
  desc?: string;
  /** 初始引导的底部补充提示 */
  hint?: string;
  /** err 态的重试按钮文案 */
  actionLabel?: string;
}>();

defineEmits<{ action: [] }>();
</script>

<template>
  <div class="state-card">
    <span v-if="kind === 'loading'" class="state-spinner" aria-hidden="true"></span>
    <span v-else class="state-icon" :class="{ 'state-icon-error': kind === 'err' }">
      <svg v-if="kind === 'err'" viewBox="0 0 24 24" aria-hidden="true">
        <circle cx="12" cy="12" r="8.2" fill="none" stroke="currentColor" stroke-width="1.7" />
        <path d="M12 7.8v5.4" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
        <circle cx="12" cy="16.4" r="0.9" fill="currentColor" />
      </svg>
      <svg v-else-if="kind === 'empty'" viewBox="0 0 24 24" aria-hidden="true">
        <circle cx="11" cy="11" r="6.5" fill="none" stroke="currentColor" stroke-width="1.7" />
        <path d="m16 16 4.5 4.5" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" />
        <path d="M8.6 11h4.8" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" />
      </svg>
      <svg v-else viewBox="0 0 24 24" aria-hidden="true">
        <path d="M9.2 17.3V7l9-1.9v10.2" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linejoin="round" />
        <circle cx="6.9" cy="17.4" r="2.3" fill="none" stroke="currentColor" stroke-width="1.7" />
        <circle cx="15.9" cy="15.4" r="2.3" fill="none" stroke="currentColor" stroke-width="1.7" />
      </svg>
    </span>
    <h2>{{ title }}</h2>
    <p v-if="desc">{{ desc }}</p>
    <button v-if="actionLabel" class="state-action" type="button" @click="$emit('action')">
      {{ actionLabel }}
    </button>
    <span v-if="hint" class="state-hint">{{ hint }}</span>
  </div>
</template>

<style scoped>
.state-card {
  min-height: 300px;
  margin-top: 20px;
  padding: 32px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 9px;
  text-align: center;
  border-radius: 18px;
  border: 1px solid var(--hairline);
  background: color-mix(in srgb, var(--bg-2) 62%, transparent);
  backdrop-filter: blur(14px);
  -webkit-backdrop-filter: blur(14px);
}
.state-card h2 {
  font-size: 19px;
  font-weight: 700;
  color: var(--text);
}
.state-card p {
  font-size: 13px;
  color: var(--text-2);
}
.state-hint {
  margin-top: 2px;
  font-size: 12px;
  color: var(--text-3);
}
.state-icon {
  width: 52px;
  height: 52px;
  margin-bottom: 6px;
  border-radius: 50%;
  display: grid;
  place-items: center;
  color: var(--accent);
  background: color-mix(in srgb, var(--accent) 12%, transparent);
  border: 1px solid color-mix(in srgb, var(--accent) 20%, transparent);
}
.state-icon svg {
  width: 24px;
  height: 24px;
}
.state-icon-error {
  color: #e0a23a;
  background: color-mix(in srgb, #e0a23a 12%, transparent);
  border-color: color-mix(in srgb, #e0a23a 22%, transparent);
}
.state-spinner {
  width: 34px;
  height: 34px;
  margin-bottom: 8px;
  border-radius: 50%;
  border: 3px solid var(--hairline);
  border-top-color: var(--accent);
  animation: ov-spin 0.8s linear infinite;
}
.state-action {
  margin-top: 8px;
  height: 32px;
  padding: 0 20px;
  border-radius: 999px;
  background: var(--accent);
  color: #fff;
  font-size: 12.5px;
  font-weight: 600;
}
.state-action:hover {
  background: var(--accent-hover);
}
@keyframes ov-spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
