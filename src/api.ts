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
}

export interface LyricsPayload {
  synced: LyricLine[] | null;
  plain: string | null;
}

export interface ImportReport {
  foldersAdded: number;
  filesCopied: number;
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
  /** 本地日期(YYYY-MM-DD)→ 当日收听秒数 */
  days: Record<string, number>;
  /** Track.id → 单曲统计 */
  tracks: Record<string, TrackStat>;
}

export const api = {
  getLibrary: () => invoke<Library>("get_library"),
  pickFolder: () => invoke<string | null>("pick_music_folder"),
  addFolder: (path: string) => invoke<void>("add_folder", { path }),
  removeFolder: (path: string) => invoke<void>("remove_folder", { path }),
  scan: () => invoke<ScanWithLibrary>("scan_library"),
  getLyrics: (id: string) => invoke<LyricsPayload | null>("get_lyrics", { id }),
  getImportDir: () => invoke<string>("get_import_dir"),
  openImportDir: () => invoke<string>("open_import_dir"),
  importPaths: (paths: string[]) => invoke<ImportReport>("import_paths", { paths }),
  importTmc: (path: string) => invoke<string>("import_tmc", { path }),
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
  statsAdd: (samples: Record<string, number>, plays: string[], day: string, daySec: number) =>
    invoke<void>("stats_add", { samples, plays, day, daySec }),
  statsClear: () => invoke<void>("stats_clear"),
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
  getResourceUsage: () => invoke<ResourceUsage>("get_resource_usage"),
};
