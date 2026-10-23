<script setup lang="ts">
import { ref } from "vue";
import Modal from "./Modal.vue";
import { acceptWelcome } from "../stores/ui";
import { api } from "../api";
import { AUTHOR_MAIL } from "../constants";

/** 三步引导:1 使用声明 → 2 感谢信 → 3 初始设置(自启动与音乐包关联,默认全部不勾) */
const step = ref<1 | 2 | 3>(1);
const wantAutostart = ref(false);
const wantPackAssoc = ref(false);
const applying = ref(false);

async function finishWelcome(): Promise<void> {
  applying.value = true;
  try {
    await api.setAutostart(wantAutostart.value);
    // 安装包默认注册了音乐包关联;用户不勾选时逐项摘除(音频格式的关联保持不变)
    if (!wantPackAssoc.value) {
      for (const ext of ["tmc", "tmcl", "tmca"]) {
        try {
          await api.setAssociation(ext, false);
        } catch (err) {
          console.error(`摘除 .${ext} 关联失败`, err);
        }
      }
    }
  } catch (err) {
    console.error("应用初始设置失败", err);
  }
  applying.value = false;
  acceptWelcome();
}
</script>

<template>
  <Modal @close="acceptWelcome">
    <Transition name="step" mode="out-in">
      <!-- 第一步:使用声明 -->
      <div v-if="step === 1" key="terms">
        <header class="head">
          <span class="logo">
            <svg viewBox="0 0 24 24">
              <!-- 迷你黑胶:与应用图标「墨夜胶片」同一几何语言,中孔透出陶土底色 -->
              <circle cx="12" cy="12" r="8.6" fill="none" stroke="#fff" stroke-width="2" />
              <circle cx="12" cy="12" r="3.4" fill="#fff" />
              <circle cx="12" cy="12" r="1.2" style="fill: var(--accent)" />
            </svg>
          </span>
          <div>
            <h2>欢迎使用 TauriMusic</h2>
            <p class="sub">首次使用前,请阅读以下使用声明</p>
          </div>
        </header>

        <ul class="terms">
          <li>
            <b>本地优先</b>
            <span>本地曲目直接播放你导入的音频文件,不会修改或移动原始文件;在线音乐提供网易云 / QQ 音乐 / 酷狗音乐 / 酷我音乐的免费曲目试听,并可保存到本地库。</span>
          </li>
          <li>
            <b>隐私</b>
            <span>不收集、不上传任何用户数据;曲库、播放记录与统计只保存在本机。在线搜索关键词与曲目 id 会发送到你选择平台的公开接口,用于获取试听、歌词与封面。</span>
          </li>
          <li>
            <b>在线内容</b>
            <span>在线试听、歌词、封面与保存入库都走各平台公开接口,只取免费给匿名听众的内容,音质也不试探付费档;VIP / 付费 / 版权受限曲目不做任何解锁,也不提供绕过手段。保存下来的曲目写进应用导入目录,与其他本地音乐一同管理。</span>
          </li>
          <li>
            <b>音乐版权</b>
            <span>请确保导入的音乐为你合法获得;导出的音乐包内容仅限个人使用,分享所产生的法律风险由使用者自行承担,与 TauriMusic 无关。</span>
          </li>
        </ul>

        <button class="cta" @click="step = 2">同意并继续</button>
      </div>

      <!-- 第二步:感谢信 -->
      <div v-else-if="step === 2" key="letter" class="letter-wrap">
        <header class="head letter-head">
          <span class="logo">
            <svg viewBox="0 0 24 24">
              <!-- 迷你黑胶:与应用图标「墨夜胶片」同一几何语言,中孔透出陶土底色 -->
              <circle cx="12" cy="12" r="8.6" fill="none" stroke="#fff" stroke-width="2" />
              <circle cx="12" cy="12" r="3.4" fill="#fff" />
              <circle cx="12" cy="12" r="1.2" style="fill: var(--accent)" />
            </svg>
          </span>
          <div>
            <h2>一封感谢信</h2>
            <p class="sub">来自开发者</p>
          </div>
        </header>

        <div class="letter">
          <p class="salut">致每一个打开 TauriMusic 的你:</p>
          <p>谢谢您愿意支持我的项目。</p>
          <p>
            它源于一个很简单的念头——音乐就该被干净、安静地播放,不被广告打扰,轻量的资源占用，理所当然的，TauriMusic属于你。
          </p>
          <p>这是我的第一个开源项目，因此希望您能对这个项目给予一些额外的宽容，我会继续努力完善它。</p>
          <p class="ps">如果您有任何好的想法和意见欢迎issue|Email。</p>
          <div class="sign">
            <span class="from">—— O_Art3mis</span>
            <a class="mail" :href="`mailto:${AUTHOR_MAIL}`">{{ AUTHOR_MAIL }}</a>
          </div>
        </div>

        <button class="cta" @click="step = 3">继续</button>
      </div>
      <div v-else key="setup" class="setup-wrap">
        <header class="head">
          <span class="logo">
            <svg viewBox="0 0 24 24">
              <!-- 迷你黑胶:与应用图标「墨夜胶片」同一几何语言,中孔透出陶土底色 -->
              <circle cx="12" cy="12" r="8.6" fill="none" stroke="#fff" stroke-width="2" />
              <circle cx="12" cy="12" r="3.4" fill="#fff" />
              <circle cx="12" cy="12" r="1.2" style="fill: var(--accent)" />
            </svg>
          </span>
          <div>
            <h2>初始设置</h2>
            <p class="sub">两项可选项,以后随时能在设置里更改</p>
          </div>
        </header>

        <div class="setup">
          <label class="opt-row">
            <input v-model="wantAutostart" type="checkbox" />
            <span class="opt-text">
              <b>开机自启动</b>
              <span>登录 Windows 后自动在后台启动 TauriMusic(托盘常驻)</span>
            </span>
          </label>
          <label class="opt-row">
            <input v-model="wantPackAssoc" type="checkbox" />
            <span class="opt-text">
              <b>关联音乐包格式</b>
              <span>双击 .tmc / .tmcl / .tmca 文件时用 TauriMusic 打开</span>
            </span>
          </label>
        </div>

        <button class="cta" :disabled="applying" @click="finishWelcome">
          {{ applying ? "正在应用…" : "完成" }}
        </button>
      </div>
    </Transition>

    <div class="dots">
      <span class="dot" :class="{ on: step === 1 }"></span>
      <span class="dot" :class="{ on: step === 2 }"></span>
      <span class="dot" :class="{ on: step === 3 }"></span>
    </div>
  </Modal>
</template>

<style scoped>
.head {
  display: flex;
  align-items: center;
  gap: 14px;
  margin-bottom: 14px;
}
.logo {
  width: 52px;
  height: 52px;
  border-radius: 13px;
  background: var(--accent);
  display: flex;
  align-items: center;
  justify-content: center;
  box-shadow: 0 4px 14px rgba(0, 0, 0, 0.25);
  flex: none;
}
.logo svg {
  width: 34px;
  height: 34px;
}
h2 {
  font-size: 18px;
  font-weight: 700;
  color: var(--text);
}
.sub {
  font-size: 12.5px;
  color: var(--text-2);
  margin-top: 3px;
}

.terms {
  display: grid;
  gap: 10px;
  margin-bottom: 16px;
}
.terms li {
  list-style: none;
  border: 1px solid var(--hairline);
  border-radius: 12px;
  padding: 11px 14px;
  display: grid;
  gap: 3px;
}
.terms b {
  font-size: 13px;
  color: var(--text);
}
.terms span {
  font-size: 12.5px;
  color: var(--text-2);
  line-height: 1.65;
}

/* ===== 感谢信 ===== */
.letter {
  border: 1px solid var(--hairline);
  border-radius: 12px;
  padding: 16px 18px;
  margin-bottom: 16px;
  font-size: 13px;
  color: var(--text);
  line-height: 1.9;
}
.letter p {
  margin-bottom: 8px;
}
.letter p:last-child {
  margin-bottom: 0;
}
.salut {
  font-weight: 600;
}
.ps {
  color: var(--text-2);
  font-size: 12.5px;
}
.sign {
  margin-top: 10px;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 2px;
}
.from {
  font-weight: 600;
}
.mail {
  font-size: 12px;
  color: var(--accent);
  text-decoration: none;
}
.mail:hover {
  text-decoration: underline;
}

/* ===== 初始设置 ===== */
.setup {
  display: grid;
  gap: 10px;
  margin-bottom: 16px;
}
.opt-row {
  display: flex;
  gap: 12px;
  align-items: flex-start;
  border: 1px solid var(--hairline);
  border-radius: 12px;
  padding: 13px 15px;
  cursor: pointer;
  transition: background 0.2s ease, border-color 0.2s ease;
}
.opt-row:hover {
  background: var(--hover);
  border-color: var(--text-3);
}
.opt-row input {
  margin-top: 3px;
  width: 15px;
  height: 15px;
  accent-color: var(--accent);
  flex: none;
}
.opt-text {
  display: grid;
  gap: 2px;
}
.opt-text b {
  font-size: 13px;
  color: var(--text);
}
.opt-text span {
  font-size: 12px;
  color: var(--text-2);
  line-height: 1.6;
}

.cta {
  width: 100%;
  height: 42px;
  border-radius: 999px;
  background: var(--accent);
  color: #fff;
  font-size: 14px;
  font-weight: 600;
  transition: filter 0.2s ease, transform 0.15s var(--ease-out-soft);
}
.cta:hover {
  filter: brightness(1.08);
}
.cta:active {
  transform: scale(0.98);
}

/* 步骤指示点 */
.dots {
  display: flex;
  justify-content: center;
  gap: 6px;
  margin-top: 12px;
}
.dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--pill-bg);
  transition: background 0.25s ease, transform 0.25s var(--ease-spring);
}
.dot.on {
  background: var(--accent);
  transform: scale(1.15);
}

/* 步骤切换过渡 */
.step-enter-active {
  transition: opacity 0.28s var(--ease-out-soft), transform 0.34s var(--ease-out-soft);
}
.step-leave-active {
  transition: opacity 0.16s ease, transform 0.16s ease;
}
.step-enter-from {
  opacity: 0;
  transform: translateX(18px);
}
.step-leave-to {
  opacity: 0;
  transform: translateX(-14px);
}
</style>
