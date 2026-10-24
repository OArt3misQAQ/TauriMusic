<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, onUpdated, ref, useId } from "vue";

const emit = defineEmits<{ close: [] }>();

const modalEl = ref<HTMLDivElement | null>(null);
/** 标题 id 供 aria-labelledby 引用:标题是各弹窗槽内容里的 h2,挂载后统一编号 */
const titleId = useId();
const labelledby = ref<string | undefined>(undefined);
/** 打开前的焦点元素:关闭时归还,键盘流不断 */
let lastActive: Element | null = null;

/** 焦点圈内可 Tab 的元素;隐藏元素(v-show/过渡残影)剔除 */
function focusables(): HTMLElement[] {
  const root = modalEl.value;
  if (!root) return [];
  return Array.from(
    root.querySelectorAll<HTMLElement>(
      'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
    ),
  ).filter((el) => el.offsetWidth > 0 || el.offsetHeight > 0);
}

/** 标题解析:About 弹窗在"关于/感谢信"两视图间切换,标题元素会换,更新后重挂 id */
function resolveTitle(): void {
  const title = modalEl.value?.querySelector<HTMLElement>("h1, h2, [data-modal-title]");
  if (title) {
    title.id = titleId;
    labelledby.value = titleId;
  } else {
    labelledby.value = undefined;
  }
}

function onKey(e: KeyboardEvent): void {
  if (e.key === "Escape") {
    emit("close");
    return;
  }
  if (e.key !== "Tab") return;
  // 简单焦点圈禁:Tab/Shift+Tab 在弹窗首尾循环,焦点不落到遮罩后的界面
  const root = modalEl.value;
  if (!root) return;
  const list = focusables();
  const active = document.activeElement;
  const inside = active instanceof HTMLElement && root.contains(active);
  if (list.length === 0) {
    // 打包中等无焦点元素的弹窗:焦点钉在容器上
    e.preventDefault();
    root.focus();
    return;
  }
  const first = list[0]!;
  const last = list[list.length - 1]!;
  if (e.shiftKey) {
    if (!inside || active === first) {
      e.preventDefault();
      last.focus();
    }
  } else if (!inside || active === last) {
    e.preventDefault();
    first.focus();
  }
}

onMounted(() => {
  lastActive = document.activeElement;
  window.addEventListener("keydown", onKey);
  void nextTick(() => {
    resolveTitle();
    // 输入型弹窗(重命名)会自己聚焦输入框:焦点已在弹窗内就不抢
    const active = document.activeElement;
    const root = modalEl.value;
    if (root && !(active instanceof HTMLElement && root.contains(active))) {
      root.focus({ preventScroll: true });
    }
  });
});
onUpdated(resolveTitle);
onUnmounted(() => {
  window.removeEventListener("keydown", onKey);
  if (lastActive instanceof HTMLElement && lastActive.isConnected) {
    lastActive.focus({ preventScroll: true });
  }
});
</script>

<template>
  <div class="overlay" @click.self="emit('close')">
    <div ref="modalEl" class="modal" role="dialog" aria-modal="true" :aria-labelledby="labelledby" tabindex="-1">
      <slot />
    </div>
  </div>
</template>

<style scoped>
.modal:focus {
  /* 容器焦点是弹窗打开时的落点,只作读屏锚点,不画焦点环(内部 Tab 导航不受影响) */
  outline: none;
}
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
