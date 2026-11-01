<script setup lang="ts">
import { computed } from "vue";
import { player } from "../stores/player";

/* 进度细线独立订阅播放位置:顶栏/迷你条其余节点不随 ~4Hz 的进度更新重渲。
   线体样式由使用方的 scoped CSS 负责(类名经 attribute fallthrough 传入) */
const progress = computed(() =>
  player.duration > 0 ? Math.min(100, (player.position / player.duration) * 100) : 0,
);
</script>

<template>
  <span :style="{ width: progress + '%' }"></span>
</template>
