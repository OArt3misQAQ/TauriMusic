<script setup lang="ts">
import { computed, ref } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import { albumByKey, enrichAlbumNetease, deleteTracks, exportAlbumTmca } from "../stores/library";
import { useConfirmableAction } from "../composables/useConfirmableAction";
import { playTracks, shuffleList } from "../stores/player";
import { ui, go, openArtist } from "../stores/ui";
import TrackList from "../components/TrackList.vue";

const album = computed(() => albumByKey(ui.albumKey ?? ""));
const years = computed(() => {
  const ys = new Set(album.value?.tracks.map((t) => t.year).filter((y): y is number => y != null) ?? []);
  return ys.size === 1 ? String([...ys][0]) : null;
});
const genres = computed(() => {
  const gs = new Set(album.value?.tracks.map((t) => t.genre).filter((g): g is string => !!g) ?? []);
  return gs.size === 1 ? [...gs][0] : null;
});
const meta = computed(() =>
  [years.value, genres.value, album.value ? `${album.value.tracks.length} 首歌曲` : null]
    .filter(Boolean)
    .join(" · "),
);

function shuffleAll(): void {
  if (!album.value) return;
  shuffleList(album.value.tracks);
}

const enriching = ref(false);
function onEnrich(): void {
  if (!album.value || enriching.value) return;
  enriching.value = true;
  void enrichAlbumNetease(album.value.key).finally(() => (enriching.value = false));
}

/** 整张专辑导出为 .tmca 专辑包(单文件,导入自动入库并归到专辑) */
function onExportAlbum(): void {
  if (!album.value) return;
  void exportAlbumTmca(album.value.key, album.value.album);
}

/** 点艺人名跳艺人页:艺人列表按 t.artist 构建,而专辑头展示的是
 *  albumArtist(可能不同)——直接传专辑头名字会因不在列表里落到排序第一人 */
function gotoArtist(): void {
  if (!album.value) return;
  openArtist(album.value.tracks[0]?.artist || album.value.artist);
}

/** 删除整张专辑(两段确认,确认后回到专辑列表) */
const { confirmingId: confirmingDelete, confirm: confirmDelete } = useConfirmableAction();

function onDeleteAlbum(): void {
  if (!album.value) return;
  const ids = album.value.tracks.map((t) => t.id);
  if (confirmDelete("__album__")) {
    // 以"专辑是否还在"决定去留:部分删除失败(文件被占用)时专辑仍残留,
    // 留在原页才能让用户看到还剩几首
    void deleteTracks(ids).then(() => {
      if (!albumByKey(ui.albumKey ?? "")) go("albums");
    });
  }
}
</script>

<template>
  <div v-if="album" class="view">
    <button class="back-btn" @click="go('albums')">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M14.5 5.5 8 12l6.5 6.5" />
      </svg>
      专辑
    </button>
    <header class="detail-head">
      <div class="cover">
        <img v-if="album.coverLarge ?? album.cover" :src="convertFileSrc(album.coverLarge ?? album.cover!)" alt="" />
        <div v-else class="ph">♪</div>
      </div>
      <div class="meta-col">
        <div class="kind">专辑</div>
        <h1>{{ album.album }}</h1>
        <div class="artist" @click="gotoArtist">{{ album.artist }}</div>
        <div class="sub">{{ meta }}</div>
        <div class="btns">
          <button class="pill pill--accent primary-pill" @click="playTracks(album.tracks, 0)">播放</button>
          <button class="pill ghost-pill" @click="shuffleAll">随机播放</button>
          <button
            class="pill ghost-pill"
            title="整张专辑打包成单个 .tmca 专辑包(7z 存储模式,打包接近拷贝速度)"
            @click="onExportAlbum"
          >
            导出专辑
          </button>
          <button
            class="pill ghost-pill"
            title="通过网易云公开接口补全封面与歌词(仅元数据,不影响本地文件播放)"
            :disabled="enriching"
            @click="onEnrich"
          >
            {{ enriching ? "匹配中…" : "在线匹配" }}
          </button>
          <button
            class="pill ghost-pill danger"
            :class="{ confirm: confirmingDelete }"
            :title="confirmingDelete ? '再次点击确认删除整张专辑(音频移入回收站)' : '删除这张专辑(音频移入回收站)'"
            @click="onDeleteAlbum"
          >
            {{ confirmingDelete ? "确认删除?" : "删除专辑" }}
          </button>
        </div>
      </div>
    </header>

    <TrackList :tracks="album.tracks" show-index virtual />
  </div>
  <div v-else class="view">
    <div class="empty">
      <h2>未找到该专辑</h2>
      <button class="pill pill--accent primary-pill" @click="go('albums')">返回专辑列表</button>
    </div>
  </div>
</template>

<style scoped>
/* 虚拟滚动:视图变为有界容器,专辑头部固定,曲目列表自身滚动 */
.view {
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.view :deep(.detail-head) {
  flex: none;
  margin-bottom: 14px;
}
.view :deep(.tl.virtual) {
  flex: 1;
  min-height: 0;
}

.detail-head {
  display: flex;
  gap: 28px;
  align-items: flex-end;
  margin-bottom: 28px;
}
.cover {
  width: 210px;
  height: 210px;
  flex: none;
  border-radius: 10px;
  overflow: hidden;
  box-shadow: var(--card-shadow);
  background: var(--bg-3);
}
.cover img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}
.cover .ph {
  width: 100%;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 56px;
  color: var(--text-3);
}

.meta-col {
  min-width: 0;
  padding-bottom: 4px;
}
.kind {
  font-size: 12px;
  font-weight: 700;
  color: var(--accent);
  letter-spacing: 0.4px;
}
h1 {
  font-size: 26px;
  font-weight: 700;
  margin-top: 4px;
}
.artist {
  font-size: 15px;
  font-weight: 600;
  color: var(--accent);
  margin-top: 6px;
  cursor: pointer;
  width: fit-content;
}
.artist:hover {
  text-decoration: underline;
}
.sub {
  font-size: 13px;
  color: var(--text-2);
  margin-top: 6px;
}
.btns {
  display: flex;
  gap: 12px;
  margin-top: 16px;
}
/* 幽灵胶囊:基类 .pill 之上只留尺寸与悬停亮度反馈(悬停不换底,
   保留"确认删除"的描边警示不被悬停底色盖掉) */
.ghost-pill {
  padding: 8px 22px;
  font-size: 14px;
  transition: filter 0.2s ease, transform 0.35s var(--ease-spring);
}
.ghost-pill:hover {
  filter: brightness(1.06);
}
/* 在线匹配的禁用态原样式不做淡化,基类的禁用变灰在此关掉 */
.ghost-pill:disabled {
  opacity: 1;
  cursor: pointer;
}
.ghost-pill.danger.confirm {
  color: var(--danger);
  border: 1px solid var(--danger);
}
</style>
