<script setup lang="ts">
import { ref, onUnmounted } from "vue";
import { MODE_LABEL, player, expandQueueToLibrary, type PlayMode } from "../stores/player";
import { lib } from "../stores/library";

// 气泡弹出方向由使用处决定:顶栏贴着窗口上沿,向上弹会被窗体裁掉看不见,
// 只有向下弹才可见;播放页控制行在窗口底部,默认向上弹
const props = withDefaults(defineProps<{ hintDir?: "above" | "below" }>(), {
  hintDir: "above",
});

/** 切换后短暂显示模式名气泡:图标变化太含蓄,需要明确的操作反馈 */
const hint = ref("");
let hintTimer = 0;

function cycle(): void {
  const order: PlayMode[] = ["seq", "loop", "one", "shuffle"];
  const i = order.indexOf(player.mode);
  const next = order[(i + 1) % order.length];
  player.mode = next;
  // 模式键切入随机 = 全库随机:上下文队列(专辑/歌单)自动扩展为整库,
  // 当前曲目无缝续播;各列表自己的「随机播放」按钮仍是列表内随机
  if (next === "shuffle") {
    const all = lib.tracks;
    if (all.length > 0 && all.length !== player.queue.length) expandQueueToLibrary(all);
  }
  hint.value = MODE_LABEL[player.mode];
  window.clearTimeout(hintTimer);
  hintTimer = window.setTimeout(() => (hint.value = ""), 1300);
}

onUnmounted(() => window.clearTimeout(hintTimer));

type Mode = PlayMode;
const icons: Record<Mode, string[]> = {
  seq: ["M4 12h11", "M12 8l4 4-4 4", "M20 5v14"],
  loop: [
    "m17 2.5 3.5 3.5-3.5 3.5",
    "M3.5 11.5v-1a4.5 4.5 0 0 1 4.5-4.5h12",
    "m7 21.5-3.5-3.5L7 14.5",
    "M20.5 12.5v1a4.5 4.5 0 0 1-4.5 4.5H4",
  ],
  one: [
    "m17 2.5 3.5 3.5-3.5 3.5",
    "M3.5 11.5v-1a4.5 4.5 0 0 1 4.5-4.5h12",
    "m7 21.5-3.5-3.5L7 14.5",
    "M20.5 12.5v1a4.5 4.5 0 0 1-4.5 4.5H4",
  ],
  shuffle: [
    "M2.5 18h1.6c1.3 0 2.6-.6 3.4-1.7l6-8.6c.8-1.1 2.1-1.7 3.4-1.7h3.1",
    "m18.5 2.5 3 3.5-3 3.5",
    "M2.5 6h1.6c1.5 0 2.9.9 3.7 2.2",
    "M20 18h-4.1c-1.3 0-2.5-.7-3.3-1.8l-.6-.8",
    "m18.5 14.5 3 3.5-3 3.5",
  ],
};
</script>

<template>
  <!-- 播放模式单键:顺序 → 列表循环 → 单曲循环 → 随机,四态互斥循环切换 -->
  <button
    class="mode-btn"
    :class="{ on: player.mode !== 'seq' }"
    :title="MODE_LABEL[player.mode]"
    @click="cycle()"
  >
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
      <path v-for="d in icons[player.mode]" :key="d" :d="d" />
      <path v-if="player.mode === 'one'" d="M10.5 9.5 12 8.6V15" stroke-width="2" />
    </svg>
    <span v-if="player.mode === 'one'" class="badge">1</span>
    <!-- 模式名气泡:按 hintDir 向上/向下浮出,1.3 秒后淡出 -->
    <Transition name="mode-hint">
      <span v-if="hint" class="mode-hint" :class="props.hintDir">{{ hint }}</span>
    </Transition>
  </button>
</template>

<style scoped>
.mode-btn {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
  color: inherit;
}
.mode-btn.on {
  color: var(--accent);
}
.mode-btn.on svg {
  color: inherit;
}
.badge {
  position: absolute;
  top: 0;
  right: -1px;
  font-size: 8px;
  font-weight: 700;
  color: var(--accent);
}
.mode-hint {
  position: absolute;
  left: 50%;
  transform: translateX(-50%);
  white-space: nowrap;
  font-size: 12px;
  font-weight: 600;
  color: var(--text);
  background: rgba(24, 24, 28, 0.88);
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: 999px;
  padding: 4px 12px;
  box-shadow: 0 6px 20px rgba(0, 0, 0, 0.35);
  pointer-events: none;
  z-index: 400;
}
/* 两个方向共用一套样式,只有锚定边不同 */
.mode-hint.above {
  bottom: calc(100% + 10px);
}
.mode-hint.below {
  top: calc(100% + 10px);
}
.mode-hint-enter-active {
  transition: opacity 0.18s ease, transform 0.22s var(--ease-spring);
}
.mode-hint-leave-active {
  transition: opacity 0.3s ease;
}
/* 入场位移朝弹出方向的反侧:向上弹从下方浮起,向下弹从上方落下 */
.mode-hint-enter-from {
  opacity: 0;
}
.mode-hint.above.mode-hint-enter-from {
  transform: translateX(-50%) translateY(4px);
}
.mode-hint.below.mode-hint-enter-from {
  transform: translateX(-50%) translateY(-4px);
}
.mode-hint-leave-to {
  opacity: 0;
}
</style>
