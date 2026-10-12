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
/* 玻璃模式下弹窗也是磨砂卡片 */
html.glass:not(.dark) .modal {
  background: rgba(255, 255, 255, 0.78);
  /* 厚玻璃 rim light:顶边受光,整体一圈微光 */
  box-shadow:
    inset 0 1px 0 rgba(255, 255, 255, 0.85),
    inset 0 0 0 1px rgba(255, 255, 255, 0.22),
    0 24px 80px rgba(0, 0, 0, 0.4);
}
html.dark.glass .modal {
  background: rgba(27, 28, 32, 0.72);
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
