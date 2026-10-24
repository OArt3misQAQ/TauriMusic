<script setup lang="ts">
import OnlineSearchPanel from "../components/online/OnlineSearchPanel.vue";
import OnlineStateCard from "../components/online/OnlineStateCard.vue";
import OnlineResultRow from "../components/online/OnlineResultRow.vue";
import { online, sourceLabel, search, loadMore } from "../stores/online";
</script>

<template>
  <div class="view">
    <div class="online-shell">
      <header class="ov-head">
        <h1>在线音乐</h1>
        <p class="sub">搜索并试听各平台免费曲目 · 可保存到本地库 · 支持正版</p>
      </header>

      <OnlineSearchPanel />

      <OnlineStateCard
        v-if="online.phase === 'err'"
        kind="err"
        title="搜索失败"
        :desc="online.errMsg"
        action-label="重试"
        @action="search"
      />
      <OnlineStateCard
        v-else-if="online.phase === 'loading' && online.results.length === 0"
        kind="loading"
        title="正在搜索…"
        :desc="`正在连接 ${sourceLabel}`"
      />
      <OnlineStateCard
        v-else-if="online.phase === 'done' && online.results.length === 0"
        kind="empty"
        :title="`没有找到“${online.keyword}”`"
        desc="换个关键词,或切换其它音源试试"
      />

      <section v-else-if="online.results.length" class="results">
        <div class="results-head">
          <h2>搜索结果</h2>
          <span>已加载 {{ online.results.length }} 首 · {{ sourceLabel }}</span>
        </div>

        <div class="ov-list">
          <OnlineResultRow v-for="(t, i) in online.results" :key="t.id" :t="t" :i="i" />
        </div>

        <button
          v-if="online.lastPage >= 0"
          class="more"
          :disabled="online.phase === 'loading'"
          @click="loadMore"
        >
          {{ online.phase === "loading" ? "加载中…" : "加载更多" }}
        </button>
        <p v-else-if="online.results.length >= 30" class="end-hint">没有更多了</p>
      </section>

      <OnlineStateCard
        v-else
        kind="idle"
        title="搜索在线音乐"
        desc="支持网易云、QQ 音乐、酷狗音乐与酷我音乐"
        hint="输入关键词,按回车或点击搜索"
      />
    </div>
  </div>
</template>

<style scoped>
.online-shell {
  width: min(100%, 1080px);
  margin: 0 auto;
}
.ov-head {
  margin-bottom: 18px;
}
.ov-head h1 {
  font-size: 30px;
  font-weight: 700;
  letter-spacing: 0.2px;
}
.sub {
  margin-top: 4px;
  font-size: 13px;
  color: var(--text-2);
}

.results-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
  margin: 22px 2px 8px;
}
.results-head h2 {
  font-size: 15px;
  font-weight: 700;
}
.results-head span {
  font-size: 11.5px;
  color: var(--text-3);
}
.ov-list {
  display: flex;
  flex-direction: column;
}
.more {
  align-self: center;
  margin: 14px 0 4px;
  padding: 7px 26px;
  border-radius: 999px;
  background: var(--pill-bg);
  border: 1px solid var(--hairline);
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text-2);
}
.more:hover:not(:disabled) {
  background: var(--hover);
  color: var(--text);
}
.end-hint {
  align-self: center;
  margin: 14px 0 4px;
  font-size: 11.5px;
  color: var(--text-3);
}
</style>
