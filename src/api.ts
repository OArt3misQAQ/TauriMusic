import { invoke } from "@tauri-apps/api/core";

export interface Track {
  id: string;
  title: string;
  artist: string;
  album: string;
  albumArtist: string;
  trackNo: number | null;
  discNo: number | null;
  year: number | null;
  genre: string | null;
  duration: number;
  path: string;
  /** 缓存后的封面小图绝对路径(256px,列表/网格用) */
  cover: string | null;
  /** 缓存后的封面大图绝对路径(512px,详情页/全屏播放页用) */
  coverLarge: string | null;
  hasLyrics: boolean;
  /** 内容 SHA-256(仅来自音乐包导入的曲目有记录,用于跨包去重) */
  sha256: string | null;
  lrcPath: string | null;
  addedAt: number;
  mtime: number;
  size: number;
  /** ===== 以下为在线曲目专用字段(本地曲目恒为 undefined) ===== */
  /** 在线音源标记:播放时 src 走在线直链而非本地文件 */
  online?: boolean;
  /** 在线来源:netease / qq / kugou / kuwo */
  onlineSource?: OnlineSource;
  /** 平台曲目 id,播放前据此取直链 */
  onlineId?: string;
  /** 已取到的播放直链(懒取,进队列时才请求) */
  onlineUrl?: string;
  /** 搜索接口给出的远程封面直链(QQ/酷狗),Rust 端落盘后返回本地路径 */
  onlineCover?: string;
}

/** 在线音源标识 */
export type OnlineSource = "netease" | "qq" | "kugou" | "kuwo";

/** 在线曲目 id 前缀:网易云沿用历史 net:,其余用来源名 */
const ONLINE_PREFIXES: Record<OnlineSource, string> = {
  netease: "net:",
  qq: "qq:",
  kugou: "kugou:",
  kuwo: "kuwo:",
};

/** 在线曲目 id 判定:统计分域与本地路径 hash 共用同一套前缀规则 */
export function isOnlineId(id: string): boolean {
  return Object.values(ONLINE_PREFIXES).some((p) => id.startsWith(p));
}

/** 在线搜索结果(各平台公开接口) */
export interface OnlineTrack {
  id: string;
  name: string;
  artist: string;
  album: string;
  duration: number;
  /** 0/8=可免费播放 1=VIP/付费 */
  fee: number;
  source: OnlineSource;
  /** 搜索接口直接给出的封面直链(网易云/酷我为空,走各自详情或占位) */
  cover: string | null;
}

/** 下载入库时附带的曲目元数据(取自搜索结果,决定内嵌标签与文件名) */
export interface OnlineDownloadMeta {
  name: string;
  artist: string;
  album: string;
  cover?: string | null;
}

/** 在线曲目元数据:在线曲目不在曲库,统计页需要随冲刷落盘才能显示榜单 */
export interface OnlineTrackMeta {
  title: string;
  artist: string;
  album: string;
}

export interface Library {
  version: number;
  folders: string[];
  tracks: Track[];
  /** 应用内回收站(删除的音乐在这里保留一段时间,可还原) */
  trash: TrashEntry[];
}

/** 回收站条目:移入应用回收站的文件与元数据快照 */
export interface TrashEntry {
  /** 曲库记录快照(path 保留原路径) */
  track: Track;
  /** 回收站目录内的音频文件路径 */
  trashedAudio: string;
  /** 回收站目录内的歌词文件路径(若有) */
  trashedLrc: string | null;
  /** 删除时间(unix 秒) */
  deletedAt: number;
}

export interface RestoreReport {
  restored: number;
  failed: number;
}

/** 音乐包(.tmcl/.tmca)导入结果 */
export interface PackImportReport {
  name: string;
  imported: number;
  /** 因内容与曲库已有曲目相同而跳过的数量 */
  skipped: number;
}

/** 音乐包预览的曲目条目(来自暂存解包的标签/清单,未经入库) */
export interface PreviewTrack {
  title: string;
  artist: string;
  album: string;
  /** 秒 */
  duration: number;
  /** 相对预览暂存根的封面路径(null = 无封面) */
  cover: string | null;
}

/** 音乐包预览结果:解包到 covers/preview 暂存后的只读元数据 */
export interface PackagePreview {
  kind: "tmc" | "tmca" | "tmcl";
  name: string;
  /** 暂存目录绝对根(封面经 asset 协议展示) */
  root: string;
  /** 暂存目录名,确认/取消后调 discardPreview 清理 */
  stagingId: string;
  tracks: PreviewTrack[];
}

export interface ScanReport {
  added: number;
  updated: number;
  removed: number;
  total: number;
  errors: number;
}

export interface LyricLine {
  timeMs: number;
  text: string;
  /** 对照译文(双语歌词:同句原文之下的中文翻译) */
  translation: string | null;
}

export interface LyricsPayload {
  synced: LyricLine[] | null;
  plain: string | null;
}

export interface ImportReport {
  foldersAdded: number;
  filesCopied: number;
  /** 其中经 ffmpeg 转码为 FLAC 的非常规格式(ape/wma 等) */
  transcoded: number;
  skipped: number;
  duplicates: number;
}

export interface NeteaseReport {
  cover: boolean;
  lyrics: number;
  /** 其中由 LRCLIB 兜底补上的数量(含在 lyrics 内) */
  lyricsLrclib: number;
  /** 其中由酷我兜底补上的数量(含在 lyrics 内) */
  lyricsKuwo: number;
  skipped: number;
}

export interface AssocState {
  ext: string;
  label: string;
  registered: boolean;
}

/** 播放列表条目:曲库引用 + 元数据快照 */
export interface PlaylistEntry {
  id: string;
  path: string;
  title: string;
  artist: string;
  album: string;
  albumArtist: string;
  duration: number;
}

export interface Playlist {
  id: string;
  name: string;
  createdAt: number;
  entries: PlaylistEntry[];
}

export interface AddToPlaylistReport {
  added: number;
  skipped: number;
  playlists: Playlist[];
}

export interface ResourceUsage {
  /** 主进程内存占用(MB) */
  memory_mb: number;
  /** CPU 占用(%),两次查询间的均值 */
  cpu: number;
}

export interface ScanWithLibrary {
  report: ScanReport;
  /** 扫描后的完整曲库(JSON 字符串) */
  library: string;
}

/** 听歌统计(仅本机 stats.json,零联网) */
export interface TrackStat {
  /** 累计收听秒数 */
  sec: number;
  /** 播放次数(≥半曲或 ≥4 分钟记一次) */
  count: number;
  /** 最后收听时刻(unix 秒) */
  last: number;
}

export interface StatsData {
  version: number;
  /** 本地日期(YYYY-MM-DD)→ 当日收听秒数(本地+在线总和) */
  days: Record<string, number>;
  /** Track.id(本地为路径 hash,在线为 `net:` 前缀)→ 单曲统计 */
  tracks: Record<string, TrackStat>;
  /** 本地日期 → 本地曲目当日秒数 */
  daysLocal: Record<string, number>;
  /** 本地日期 → 在线曲目当日秒数 */
  daysOnline: Record<string, number>;
  /** 在线曲目 id → 展示用元数据 */
  onlineMeta: Record<string, OnlineTrackMeta>;
}

/** 同一首歌的歌词请求去重:切歌时播放页与 SMTC 桥会各拉一次,
 *  后端每次都要重新探测音频文件读盘——同 id 共享同一 Promise 只打一遍。
 *  请求落定后即清除缓存:在线补译等操作会更新歌词文件,陈旧缓存会让
 *  「不切歌重开播放页」永远拿到补译前的旧词 */
let lyricsInFlight: { id: string; payload: Promise<LyricsPayload | null> } | null = null;

function getLyricsDedup(id: string): Promise<LyricsPayload | null> {
  if (lyricsInFlight?.id === id) return lyricsInFlight.payload;
  const p = invoke<LyricsPayload | null>("get_lyrics", { id });
  lyricsInFlight = { id, payload: p };
  // 落定(成功或失败)即作废:成功结果不再复用,避免盖掉之后的歌词文件更新
  const clear = () => {
    if (lyricsInFlight?.payload === p) lyricsInFlight = null;
  };
  p.then(clear, clear);
  return p;
}

export const api = {
  getLibrary: () => invoke<Library>("get_library"),
  pickFolder: () => invoke<string | null>("pick_music_folder"),
  addFolder: (path: string) => invoke<void>("add_folder", { path }),
  removeFolder: (path: string) => invoke<void>("remove_folder", { path }),
  scan: () => invoke<ScanWithLibrary>("scan_library"),
  getLyrics: getLyricsDedup,
  getImportDir: () => invoke<string>("get_import_dir"),
  openImportDir: () => invoke<string>("open_import_dir"),
  importPaths: (paths: string[]) => invoke<ImportReport>("import_paths", { paths }),
  importTmc: (path: string) => invoke<PackImportReport>("import_tmc", { path }),
  importTmca: (path: string) => invoke<PackImportReport>("import_tmca", { path }),
  pickTmcFile: () => invoke<string | null>("pick_tmc_file"),
  pickAudioFiles: () => invoke<string[] | null>("pick_audio_files"),
  pickExportDir: () => invoke<string | null>("pick_export_dir"),
  // 两阶段导出:先打包到 Temp 暂存区(弹窗显示进度),就绪后用户点「导出」再落位完整文件
  makeStageDir: () => invoke<string>("make_stage_dir"),
  stageTmc: (id: string, staging: string) => invoke<string>("stage_tmc", { id, staging }),
  stageTmcl: (playlistId: string, staging: string) =>
    invoke<string>("stage_tmcl", { playlistId, staging }),
  stageTmca: (albumKey: string, staging: string) =>
    invoke<string>("stage_tmca", { albumKey, staging }),
  placeStaged: (files: string[], destDir: string) =>
    invoke<string[]>("place_staged", { files, destDir }),
  cleanupStage: (dir: string) => invoke<void>("cleanup_stage", { dir }),
  neteaseEnrichAlbum: (albumKey: string) => invoke<NeteaseReport>("netease_enrich_album", { albumKey }),
  getAssociations: () => invoke<AssocState[]>("get_associations"),
  setAssociation: (ext: string, enable: boolean) => invoke<void>("set_association", { ext, enable }),
  getAutostart: () => invoke<boolean>("get_autostart"),
  setAutostart: (enable: boolean) => invoke<void>("set_autostart", { enable }),
  getStats: () => invoke<StatsData>("stats_get"),
  /** force=true 跳过后端落盘节流立即写盘(隐藏到托盘/退出前/关统计开关时用) */
  statsAdd: (
    samples: Record<string, number>,
    plays: string[],
    day: string,
    daySec: number,
    onlineMeta: Record<string, OnlineTrackMeta>,
    force = false,
  ) => invoke<void>("stats_add", { samples, plays, day, daySec, onlineMeta, force }),
  statsClear: () => invoke<void>("stats_clear"),
  /** 系统媒体控制(SMTC)与托盘菜单的状态推送 */
  smtcSetTrack: (title: string, artist: string, cover: string | null, duration: number) =>
    invoke<void>("smtc_set_track", { title, artist, cover, duration }),
  smtcSetStatus: (playing: boolean) => invoke<void>("smtc_set_status", { playing }),
  smtcSetPosition: (position: number, duration: number) =>
    invoke<void>("smtc_set_position", { position, duration }),
  deleteTracks: (ids: string[]) =>
    invoke<{ deleted: number; failed: number }>("delete_tracks", { ids }),
  /** 还原回收站条目;ids 为空数组表示还原全部 */
  restoreTracks: (ids: string[]) => invoke<RestoreReport>("restore_tracks", { ids }),
  /** 彻底删除回收站条目;ids 为空数组表示清空全部 */
  purgeTrash: (ids: string[]) => invoke<number>("purge_trash", { ids }),
  createPlaylist: (name: string) => invoke<Playlist[]>("create_playlist", { name }),
  renamePlaylist: (id: string, name: string) => invoke<Playlist[]>("rename_playlist", { id, name }),
  deletePlaylist: (id: string) => invoke<Playlist[]>("delete_playlist", { id }),
  addTracksToPlaylist: (playlistId: string, trackIds: string[]) =>
    invoke<AddToPlaylistReport>("add_tracks_to_playlist", { playlistId, trackIds }),
  removePlaylistEntry: (playlistId: string, index: number) =>
    invoke<Playlist[]>("remove_playlist_entry", { playlistId, index }),
  movePlaylistEntry: (playlistId: string, from: number, to: number) =>
    invoke<Playlist[]>("move_playlist_entry", { playlistId, from, to }),
  importPlaylistTmcl: (path: string) => invoke<PackImportReport>("import_playlist_tmcl", { path }),
  /** 解析音乐包内容供预览:只读暂存,不写库 */
  inspectPackage: (path: string) => invoke<PackagePreview>("inspect_package", { path }),
  /** 清理一个预览暂存目录 */
  discardPreview: (id: string) => invoke<void>("discard_preview", { id }),
  /** 拉走并清空待预览的音乐包路径(启动期前端未就绪时后端先入队) */
  takePendingImports: () => invoke<string[]>("take_pending_imports"),
  /** 显示/隐藏桌面歌词小窗 */
  showLyricsWindow: () => invoke<void>("show_lyrics_window"),
  hideLyricsWindow: () => invoke<void>("hide_lyrics_window"),
  getResourceUsage: () => invoke<ResourceUsage>("get_resource_usage"),
  /** 在线搜索(按来源分发;免费曲目可直接播放) */
  onlineSearch: (source: OnlineSource, keyword: string, page: number) =>
    invoke<OnlineTrack[]>("online_search", { source, keyword, page }),
  /** 在线曲目播放直链(版权/VIP 受限曲目 reject;后端已做域名白名单校验。
   *  码率固定取各平台免费上限:网易云/酷狗 320k,QQ 与酷我 128k) */
  onlinePlayUrl: (source: OnlineSource, id: string) =>
    invoke<string>("online_play_url", { source, id }),
  /** 在线曲目保存到本地库:取链→下载→写标签/封面/歌词→走散装导入管线。
   *  付费曲目在取链一步就 reject,与播放同一道闸 */
  onlineDownload: (source: OnlineSource, id: string, meta: OnlineDownloadMeta) =>
    invoke<ImportReport>("online_download", { source, id, meta }),
  /** 在线曲目封面:Rust 端落盘到封面缓存后返回本地路径(无图返回 null) */
  onlineCover: (source: OnlineSource, id: string, coverUrl: string | null) =>
    invoke<string | null>("online_cover", { source, id, coverUrl }),
};

/** 在线曲目在播放队列里的完整 id(来源前缀 + 平台 id):
 *  「正在播放」高亮比对等场景复用,避免各处重复拼前缀 */
export function onlineTrackId(t: OnlineTrack): string {
  return `${ONLINE_PREFIXES[t.source]}${t.id}`;
}

/** 在线搜索结果 → 播放队列 Track:id 加来源前缀与本地路径 hash 空间隔离,
 *  path 留空——播放 src 走在线直链,歌词/封面等本地探测对它一律静默落空 */
export function onlineToTrack(t: OnlineTrack): Track {
  return {
    id: onlineTrackId(t),
    title: t.name,
    artist: t.artist,
    album: t.album || "在线音乐",
    albumArtist: "",
    trackNo: null,
    discNo: null,
    year: null,
    genre: null,
    duration: t.duration,
    path: "",
    cover: null,
    coverLarge: null,
    hasLyrics: false,
    sha256: null,
    lrcPath: null,
    addedAt: 0,
    mtime: 0,
    size: 0,
    online: true,
    onlineSource: t.source,
    onlineId: t.id,
    onlineCover: t.cover ?? undefined,
  };
}
