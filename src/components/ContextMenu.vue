<script setup lang="ts">
import { computed, onMounted, onUnmounted } from "vue";
import { ctx, closeCtx } from "../stores/context";
import { isMobile } from "../platform";

function run(index: number): void {
  const item = ctx.items?.[index];
  closeCtx();
  item?.action();
}

function onKey(e: KeyboardEvent): void {
  if (e.key === "Escape") closeCtx();
}

function onResize(): void {
  closeCtx();
}

function onDocClick(e: MouseEvent): void {
  if (!(e.target as HTMLElement | null)?.closest(".menu")) closeCtx();
}

onMounted(() => {
  window.addEventListener("keydown", onKey);
  window.addEventListener("resize", onResize);
  window.addEventListener("blur", onResize);
  window.addEventListener("click", onDocClick);
});
onUnmounted(() => {
  window.removeEventListener("keydown", onKey);
  window.removeEventListener("resize", onResize);
  window.removeEventListener("blur", onResize);
  window.removeEventListener("click", onDocClick);
});

/** 手机改成底部操作卡片,不再按触点浮层;桌面保持原位右键菜单 */
const style = computed(() =>
  isMobile
    ? { left: "0", right: "0", bottom: "0", top: "auto" }
    : { left: `${ctx.x}px`, top: `${ctx.y}px` },
);
</script>

<template>
  <Transition name="ctx">
    <div v-if="ctx.items" class="menu" :style="style">
      <template v-for="(item, i) in ctx.items" :key="item.label">
        <button class="item" :class="{ danger: item.danger }" @click="run(i)">
          <svg v-if="item.icon === 'play'" viewBox="0 0 24 24"><path d="M8.6 6.5v11L18 12z" fill="currentColor" /></svg>
          <svg v-else-if="item.icon === 'album'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
            <circle cx="12" cy="12" r="8.2" />
            <circle cx="12" cy="12" r="2.1" fill="currentColor" stroke="none" />
          </svg>
          <svg v-else-if="item.icon === 'artist'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round">
            <circle cx="12" cy="8.4" r="3.4" />
            <path d="M5.6 19c.8-3.3 3.3-4.9 6.4-4.9s5.6 1.6 6.4 4.9" />
          </svg>
          <svg v-else-if="item.icon === 'export'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 13.5V4M8.5 7.5 12 4l3.5 3.5" />
            <path d="M5 15v2.6c0 .77.63 1.4 1.4 1.4h11.2c.77 0 1.4-.63 1.4-1.4V15" />
          </svg>
          <svg v-else-if="item.icon === 'playlist'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round">
            <path d="M4 6.5h11M4 12h11M4 17.5h7" />
            <circle cx="18.5" cy="15.5" r="2.2" />
            <path d="M20.7 15.5V9l-3.4.9" />
          </svg>
          <svg v-else-if="item.icon === 'save'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 4.6v9.2m0 0 3.4-3.4M12 13.8l-3.4-3.4" />
            <path d="M5.4 18.4h13.2" />
          </svg>
          <svg v-else-if="item.icon === 'edit'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
            <path d="M14.5 5.5l4 4L8 20H4v-4z" />
            <path d="m12.5 7.5 4 4" />
          </svg>
          <svg v-else-if="item.icon === 'delete'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <path d="M4.5 7h15M9.5 7V5.2c0-.66.54-1.2 1.2-1.2h2.6c.66 0 1.2.54 1.2 1.2V7M6.5 7l.8 11.3c.06.77.7 1.2 1.4 1.2h6.6c.7 0 1.34-.43 1.4-1.2L17.5 7" />
            <path d="M10 11v5M14 11v5" />
          </svg>
          <span>{{ item.label }}</span>
        </button>
      </template>
    </div>
  </Transition>
</template>

<style scoped>
.menu {
  position: fixed;
  z-index: 500;
  min-width: 190px;
  background: var(--bg);
  border: 1px solid var(--hairline);
  border-radius: 12px;
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.35);
  padding: 6px;
  user-select: none;
}
html.glass:not(.dark) .menu {
  background: rgba(255, 255, 255, 0.85);
  backdrop-filter: blur(24px) saturate(1.5);
}
html.dark.glass .menu {
  background: rgba(30, 30, 36, 0.85);
  backdrop-filter: blur(24px) saturate(1.5);
}
/* 边缘透镜折射与其他玻璃面一致 */
@supports (backdrop-filter: url("#glass-refract")) {
  html.glass:not(.dark) .menu {
    backdrop-filter: blur(18px) saturate(1.5) url("#glass-refract");
    -webkit-backdrop-filter: blur(18px) saturate(1.5) url("#glass-refract");
  }
  html.dark.glass .menu {
    backdrop-filter: blur(22px) saturate(1.4) url("#glass-refract");
    -webkit-backdrop-filter: blur(22px) saturate(1.4) url("#glass-refract");
  }
}

.item {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  height: 32px;
  padding: 0 10px;
  border-radius: 8px;
  font-size: 13px;
  color: var(--text);
  text-align: left;
  transition: background 0.15s ease;
}
.item:hover {
  background: var(--hover);
}
.item svg {
  width: 15px;
  height: 15px;
  flex: none;
  color: var(--text-2);
}
.item.danger {
  color: var(--danger);
}
.item.danger svg {
  color: inherit;
}

.ctx-enter-active {
  transition: opacity 0.16s ease, transform 0.22s var(--ease-spring);
}
.ctx-leave-active {
  transition: opacity 0.12s ease;
}
.ctx-enter-from {
  opacity: 0;
  transform: scale(0.94);
}
.ctx-leave-to {
  opacity: 0;
}

/* 手机:贴底操作卡片,盖住 Tab 栏并让开手势条,行高按触屏放大 */
:root.mobile .menu {
  min-width: 0;
  max-height: 70vh;
  overflow-y: auto;
  padding: 8px 8px calc(8px + var(--tm-safe-bottom, 0px));
  border-radius: 18px 18px 0 0;
  box-shadow: 0 -8px 40px rgba(0, 0, 0, 0.3);
}
:root.mobile .item {
  height: 48px;
  padding: 0 14px;
  gap: 12px;
  border-radius: 12px;
  font-size: 15px;
}
:root.mobile .item svg {
  width: 18px;
  height: 18px;
}
:root.mobile .ctx-enter-from,
:root.mobile .ctx-leave-to {
  transform: translateY(20px);
}
</style>
