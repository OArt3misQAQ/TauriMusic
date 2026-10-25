<script setup lang="ts">
import { lib } from "../stores/library";
import { statsEnabled } from "../stores/stats";
import { go, ui } from "../stores/ui";
import BrandMark from "../components/BrandMark.vue";
</script>

<template>
  <div class="view mine">
    <header class="hero">
      <span class="hero-logo"><BrandMark /></span>
      <div class="hero-copy">
        <h1>我的</h1>
        <p>TauriMusic · {{ lib.tracks.length }} 首本地歌曲 · {{ lib.playlists.length }} 个歌单</p>
      </div>
    </header>

    <section class="share-card">
      <span class="share-ico">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
          <circle cx="18" cy="5" r="2.6" />
          <circle cx="6" cy="12" r="2.6" />
          <circle cx="18" cy="19" r="2.6" />
          <path d="m8.4 10.8 7.2-4.2M8.4 13.2l7.2 4.2" />
        </svg>
      </span>
      <div class="share-copy">
        <b>从系统分享添加音乐</b>
        <span>在文件管理器或其它 App 里把音频分享给 TauriMusic,导入后直接播放。</span>
      </div>
    </section>

    <nav class="group" aria-label="我的功能">
      <button v-if="statsEnabled" class="row" @click="go('stats')">
        <span class="row-ico" style="--tint: #0a84ff">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round">
            <path d="M5 20v-8M12 20V4M19 20v-5" />
          </svg>
        </span>
        <span class="row-label">听歌统计</span>
        <svg class="chev" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
          <path d="m9 5 7 7-7 7" />
        </svg>
      </button>

      <button class="row" @click="go('playlists')">
        <span class="row-ico" style="--tint: #ff375f">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round">
            <path d="M4 6.5h11M4 12h11M4 17.5h7" />
            <circle cx="18.5" cy="15.5" r="2.2" />
            <path d="M20.7 15.5V9l-3.4.9" />
          </svg>
        </span>
        <span class="row-label">播放列表</span>
        <span class="row-value">{{ lib.playlists.length }}</span>
        <svg class="chev" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
          <path d="m9 5 7 7-7 7" />
        </svg>
      </button>

      <button class="row" @click="go('trash')">
        <span class="row-ico" style="--tint: #98989d">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <path d="M4.5 7h15M9.5 7V5.2c0-.66.54-1.2 1.2-1.2h2.6c.66 0 1.2.54 1.2 1.2V7M6.5 7l.8 11.3c.06.77.7 1.2 1.4 1.2h6.6c.7 0 1.34-.43 1.4-1.2L17.5 7" />
          </svg>
        </span>
        <span class="row-label">回收站</span>
        <svg class="chev" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
          <path d="m9 5 7 7-7 7" />
        </svg>
      </button>

      <button class="row" @click="go('settings')">
        <span class="row-ico" style="--tint: #8e8e93">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round">
            <path d="M5 7.5h7M16.5 7.5H19M5 12h3M11.5 12H19M5 16.5h7M16.5 16.5H19" />
            <circle cx="14" cy="7.5" r="2" /><circle cx="9.5" cy="12" r="2" /><circle cx="14" cy="16.5" r="2" />
          </svg>
        </span>
        <span class="row-label">设置</span>
        <svg class="chev" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
          <path d="m9 5 7 7-7 7" />
        </svg>
      </button>

      <button class="row" @click="ui.aboutOpen = true">
        <span class="row-ico" style="--tint: #64d2ff">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round">
            <circle cx="12" cy="12" r="8.3" />
            <path d="M12 11v5.4" />
            <circle cx="12" cy="7.8" r="1.05" fill="currentColor" stroke="none" />
          </svg>
        </span>
        <span class="row-label">关于</span>
        <svg class="chev" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round">
          <path d="m9 5 7 7-7 7" />
        </svg>
      </button>
    </nav>

    <p class="foot">TauriMusic · Android</p>
  </div>
</template>

<style scoped>
.mine {
  padding-top: 8px;
}
.hero {
  display: flex;
  align-items: center;
  gap: 14px;
  margin-bottom: 16px;
}
.hero-logo {
  width: 54px;
  height: 54px;
  flex: none;
  border-radius: 16px;
  background: #1b1613;
  display: flex;
  align-items: center;
  justify-content: center;
  box-shadow: 0 8px 22px color-mix(in srgb, var(--accent) 28%, transparent);
}
.hero-logo svg {
  width: 34px;
  height: 34px;
}
.hero-copy h1 {
  font-size: 28px;
  font-weight: 700;
  letter-spacing: 0.2px;
}
.hero-copy p {
  margin-top: 3px;
  font-size: 11.5px;
  color: var(--text-2);
}
.share-card {
  display: flex;
  gap: 12px;
  align-items: flex-start;
  padding: 14px;
  margin-bottom: 18px;
  border-radius: 16px;
  border: 1px solid var(--hairline);
  background:
    radial-gradient(120% 140% at 0% 0%, color-mix(in srgb, var(--accent) 14%, transparent), transparent 62%),
    color-mix(in srgb, var(--bg-2) 76%, transparent);
}
.share-ico {
  width: 34px;
  height: 34px;
  flex: none;
  border-radius: 10px;
  background: color-mix(in srgb, var(--accent) 14%, transparent);
  color: var(--accent);
  display: flex;
  align-items: center;
  justify-content: center;
}
.share-ico svg {
  width: 18px;
  height: 18px;
}
.share-copy {
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.share-copy b {
  font-size: 13px;
  color: var(--text);
}
.share-copy span {
  font-size: 11.5px;
  line-height: 1.55;
  color: var(--text-2);
}
.group {
  display: flex;
  flex-direction: column;
  border-radius: 16px;
  border: 1px solid var(--hairline);
  background: color-mix(in srgb, var(--bg-2) 72%, transparent);
  overflow: hidden;
}
.row {
  display: flex;
  align-items: center;
  gap: 12px;
  min-height: 54px;
  padding: 0 14px;
  text-align: left;
  color: var(--text);
  transition: background 0.18s ease;
}
.row + .row {
  border-top: 1px solid var(--hairline);
}
.row:active {
  background: var(--active);
}
.row-ico {
  width: 28px;
  height: 28px;
  flex: none;
  border-radius: 8px;
  background: color-mix(in srgb, var(--tint, var(--accent)) 16%, transparent);
  color: var(--tint, var(--accent));
  display: flex;
  align-items: center;
  justify-content: center;
}
.row-ico svg {
  width: 16px;
  height: 16px;
}
.row-label {
  flex: 1;
  font-size: 14px;
}
.row-value {
  font-size: 12px;
  color: var(--text-3);
}
.chev {
  width: 15px;
  height: 15px;
  flex: none;
  color: var(--text-3);
}
.foot {
  margin: 22px 0 8px;
  text-align: center;
  font-size: 10.5px;
  color: var(--text-3);
}
</style>
