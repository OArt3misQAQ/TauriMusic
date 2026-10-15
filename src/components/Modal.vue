<script setup lang="ts">
import { onMounted, onUnmounted } from "vue";

const emit = defineEmits<{ close: [] }>();

function onKey(e: KeyboardEvent): void {
  if (e.key === "Escape") emit("close");
}
onMounted(() => window.addEventListener("keydown", onKey));
onUnmounted(() => window.removeEventListener("keydown", onKey));
</script>

<template>
  <div class="overlay" @click.self="emit('close')">
    <div class="modal">
      <slot />
    </div>
  </div>
</template>

<style scoped>
.overlay {
  position: absolute;
  inset: 0;
  z-index: 300;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.45);
  backdrop-filter: blur(6px);
  -webkit-backdrop-filter: blur(6px);
}
.modal {
  position: relative;
  width: 460px;
  max-width: 88%;
  max-height: 82%;
  overflow-y: auto;
  background: var(--bg);
  border: 1px solid var(--hairline);
  border-radius: 16px;
  box-shadow: 0 24px 80px rgba(0, 0, 0, 0.4);
  padding: 20px 22px;
}
/* 玻璃模式下弹窗也是磨砂卡片。
   基底必须先给纯 blur 磨砂:url(#glass-refract) 引用一旦失效(滤镜 SVG 被
   HMR/重构挪动等),整条 backdrop-filter 会被静默丢弃——没有这层兜底,
   弹窗就直接透底变「透明弹窗」(侧栏/顶栏/右键菜单都有兜底,唯独这里没有,
   这是弹窗玻璃反复出事的结构性根因)。@supports 命中后再升级为带折射的完整链 */
html.glass:not(.dark) .modal {
  background: rgba(255, 255, 255, 0.78);
  backdrop-filter: blur(24px) saturate(1.6);
  -webkit-backdrop-filter: blur(24px) saturate(1.6);
  /* 厚玻璃 rim light:顶边受光,整体一圈微光 */
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.85),
    inset 0 0 0 1px rgba(255, 255, 255, 0.22),
    0 24px 80px rgba(0, 0, 0, 0.4);
}
html.dark.glass .modal {
  background: rgba(27, 28, 32, 0.72);
  backdrop-filter: blur(28px) saturate(1.4);
  -webkit-backdrop-filter: blur(28px) saturate(1.4);
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.18),
    inset 0 0 0 1px rgba(255, 255, 255, 0.06),
    0 24px 80px rgba(0, 0, 0, 0.4);
}
/* 边缘透镜折射:中心清晰、边缘弯折,与侧栏/顶栏同一套折射滤镜 */
@supports (backdrop-filter: url("#glass-refract")) {
  html.glass:not(.dark) .modal {
    backdrop-filter: blur(24px) saturate(1.6) url("#glass-refract");
    -webkit-backdrop-filter: blur(24px) saturate(1.6) url("#glass-refract");
  }
  html.dark.glass .modal {
    backdrop-filter: blur(28px) saturate(1.4) url("#glass-refract");
    -webkit-backdrop-filter: blur(28px) saturate(1.4) url("#glass-refract");
  }
}
</style>
