import { computed, reactive, shallowReactive } from "vue";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import {
  api,
  type PackagePreview,
  type Playlist,
  type Track,
  type TrashEntry,
} from "../api";
import { purgeDeleted } from "./player";

export interface Album {
  /** albumArtist + album 组成的唯一键 */
  key: string;
  album: string;
  artist: string;
  year: number | null;
  /** 封面小图(列表网格用) */
  cover: string | null;
  /** 封面大图(详情页头部用) */
  coverLarge: string | null;
  tracks: Track[];
}

interface LibState {
  tracks: Track[];
  folders: string[];
  playlists: Playlist[];
  /** 应用内回收站(删除的音乐在这里保留,可还原) */
  trash: TrashEntry[];
  loaded: boolean;
  /** 启动读库失败(区别于空库,给用户重试入口) */
  loadError: string | null;
  scanning: boolean;
  scanCurrent: number;
  scanTotal: number;
  /** 应用自带的导入目录绝对路径 */
  importDir: string;
  /** 最近一次拖拽导入的结果提示(自动消失) */
  importStatus: string;
  /** 全部在线匹配进行中 */
  matching: boolean;
}

export const lib = shallowReactive<LibState>({
  tracks: [],
  folders: [],
  playlists: [],
  trash: [],
  loaded: false,
  loadError: null,
  scanning: false,
  scanCurrent: 0,
  scanTotal: 0,
  importDir: "",
  importStatus: "",
  matching: false,
});

let importStatusTimer = 0;

/** 应用后端返回的曲库数据(曲目/文件夹/播放列表统一入口) */
export function applyLibraryData(data: {
  tracks: Track[];
  folders: string[];
  playlists?: Playlist[];
  trash?: TrashEntry[];
}): void {
  lib.tracks = data.tracks;
  lib.folders = data.folders;
  if (data.playlists) lib.playlists = data.playlists;
  if (data.trash) lib.trash = data.trash;
}

let listenersInited = false;

/** 注册 Tauri 事件监听(仅一次;HMR/重复调用不会叠加) */
function initLibraryListeners(): void {
  if (listenersInited) return;
  listenersInited = true;
  void listen<{ current: number; total: number }>("scan-progress", (e) => {
    lib.scanCurrent = e.payload.current;
    lib.scanTotal = e.payload.total;
  });

  // 文件关联"打开方式"打开音乐包 → 主窗口弹出预览弹窗
  // (启动期错过的路径由 take_pending_imports 兜底,那里也会走到这里)
  void listen("import-request", async () => {
    try {
      const pending = await api.takePendingImports();
      if (pending.length) void openImportPreview(pending);
    } catch (err) {
      console.error("拉取待导入音乐包失败", err);
    }
  });

  // 后端处理"打开方式"/命令行导入完成后刷新曲库,并对新增曲目自动在线匹配
  void listen("library-changed", async () => {
    try {
      const before = new Set(lib.tracks.map((t) => t.id));
      const data = await api.getLibrary();
      applyLibraryData(data);
      await enrichNewTracks(before, "已导入");
    } catch (err) {
      console.error("刷新曲库失败", err);
    }
  });

  // 命令行/双击"打开方式"导入的失败清单(后端聚合)
  void listen<string>("import-error", (e) => {
    flashStatus(`导入失败: ${e.payload}`);
  });

  // TMCL 大包的写入进度(后端按资源数上报,驱动导出弹窗里的细进度条)
  void listen<{ done: number; total: number }>("export-progress", (e) => {
    exportState.assetDone = e.payload.done;
    exportState.assetTotal = e.payload.total;
  });
}

export async function initLibrary(): Promise<void> {
  initLibraryListeners();
  // 启动期文件关联打开的音乐包:后端先入队,前端就绪后拉取并弹预览
  try {
    const pending = await api.takePendingImports();
    if (pending.length) void openImportPreview(pending);
  } catch (err) {
    console.error("拉取待导入音乐包失败", err);
  }
  try {
    const data = await api.getLibrary();
    applyLibraryData(data);
    lib.loadError = null;
  } catch (err) {
    console.error("读取曲库失败", err);
    lib.loadError = String(err);
  } finally {
    lib.loaded = true;
  }

  try {
    lib.importDir = await api.getImportDir();
  } catch (err) {
    console.error("获取导入目录失败", err);
  }

  // 读库失败时提供重试;成功才走启动扫描
  if (lib.loadError) return;
  if (lib.folders.length > 0) {
    // 手动放进音乐目录的新文件没有经过导入入口,启动扫描后对新增曲目自动在线匹配;
    // before 为上次退出时的曲库,匹配失败的旧曲目不会重复请求
    const before = new Set(lib.tracks.map((t) => t.id));
    await rescan();
    await enrichNewTracks(before, "曲库已同步", true);
  }
}

/** 读库失败后的重试入口 */
export async function reloadLibrary(): Promise<void> {
  lib.loaded = false;
  lib.loadError = null;
  await initLibrary();
}

/** 监听文件拖进窗口:.tmca 专辑包/.tmc 音乐包走解包导入,文件夹登记扫描,散装音频/歌词复制进导入目录 */
export async function initDragImport(): Promise<void> {
  try {
    await getCurrentWebview().onDragDropEvent((e) => {
      if (e.payload.type === "drop" && e.payload.paths.length) {
        void importDropped(e.payload.paths);
      }
    });
  } catch (err) {
    console.error("拖拽导入初始化失败", err);
  }
}

/** 状态条提示(6 秒自动消失);其他模块也需要报错反馈,故导出 */
export function flashStatus(text: string): void {
  lib.importStatus = text;
  window.clearTimeout(importStatusTimer);
  importStatusTimer = window.setTimeout(() => (lib.importStatus = ""), 6000);
}

/* ===== 音乐包导入预览:先解析展示包内曲目,确认后才正式导入 ===== */

/** 音乐包扩展名 */
const PACKAGE_EXTS = [".tmc", ".tmca", ".tmcl"];

/** 预览弹窗状态:preview(展示曲目)→ importing(写库中)→ done(结果汇总) */
export const importPreview = reactive({
  open: false,
  phase: "preview" as "preview" | "importing" | "done",
  packages: [] as (PackagePreview & { path: string; checked: boolean })[],
  /** 正在导入第几个包 */
  current: 0,
  /** 当前包的导入结果行(驱动结果清单) */
  status: "",
  results: [] as string[],
  /** 本次实际新导入的歌曲(重扫后与预览前快照做差) */
  newTracks: [] as { title: string; artist: string; cover: string | null }[],
  error: "",
  summary: "",
});

/** 预览打开时的曲库快照:确认导入后据此判定哪些是新增曲目做在线补全 */
let previewBeforeIds = new Set<string>();

/** 打开导入预览:音乐包逐个解析(暂存解包,不写库);散装文件保持原直接导入通道 */
export async function openImportPreview(paths: string[]): Promise<void> {
  const packs = paths.filter((p) =>
    PACKAGE_EXTS.some((e) => p.toLowerCase().endsWith(e)),
  );
  const rest = paths.filter(
    (p) => !PACKAGE_EXTS.some((e) => p.toLowerCase().endsWith(e)),
  );
  if (rest.length) void importDropped(rest);
  if (!packs.length) return;
  previewBeforeIds = new Set(lib.tracks.map((t) => t.id));
  importPreview.open = true;
  importPreview.phase = "preview";
  importPreview.packages = [];
  importPreview.current = 0;
  importPreview.results = [];
  importPreview.error = "";
  importPreview.summary = "";
  const loaded: (PackagePreview & { path: string; checked: boolean })[] = [];
  const errors: string[] = [];
  for (const p of packs) {
    try {
      loaded.push({ ...(await api.inspectPackage(p)), path: p, checked: true });
    } catch (err) {
      console.error("解析音乐包失败", err);
      const name = p.split(/[\\/]/).pop() ?? p;
      errors.push(`${name}: ${String(err)}`);
    }
  }
  if (errors.length) importPreview.error = errors.join(" · ");
  importPreview.packages = loaded;
  if (!loaded.length) {
    importPreview.summary = "没有可导入的音乐包";
    importPreview.phase = "done";
  }
}

/** 确认导入:只导入勾选的包(解包到导入目录 + 扫描入库);
 *  在线补全走网络,放后台跑——绝不让「导入完成」等网络 */
export async function confirmPackageImport(): Promise<void> {
  if (importPreview.phase !== "preview") return;
  const selected = importPreview.packages.filter((p) => p.checked);
  if (!selected.length) return;
  importPreview.phase = "importing";
  const parts: string[] = [];
  let failed = 0;
  for (let i = 0; i < selected.length; i++) {
    importPreview.current = i + 1;
    const p = selected[i];
    try {
      const r =
        p.kind === "tmca"
          ? await api.importTmca(p.path)
          : p.kind === "tmcl"
            ? await api.importPlaylistTmcl(p.path)
            : await api.importTmc(p.path);
      const skip = r.skipped ? ` · 跳过 ${r.skipped} 首重复` : "";
      const label = p.kind === "tmca" ? "专辑" : p.kind === "tmcl" ? "歌单" : "单曲";
      const line =
        r.imported > 0
          ? `${label}「${r.name}」导入完成${skip}`
          : `${label}「${r.name}」内容与曲库重复,已跳过${skip}`;
      importPreview.status = line;
      importPreview.results.push(line);
      parts.push(`${label}「${p.name}」${skip}`);
    } catch (err) {
      failed++;
      const line = `「${p.name}」导入失败: ${String(err)}`;
      importPreview.results.push(line);
      parts.push(`「${p.name}」失败`);
    }
  }
  // 扫描入库一完成就切「导入完成」;在线补全是网络活,放后台绝不等它
  await rescan();
  // 与预览前的曲库快照做差:列出本次真正新导入的歌曲
  importPreview.newTracks = lib.tracks
    .filter((t) => !previewBeforeIds.has(t.id))
    .map((t) => ({ title: t.title, artist: t.artist, cover: t.cover }));
  importPreview.summary = parts.join(" · ") + (failed ? ` · ${failed} 个失败` : "");
  importPreview.phase = "done";
  void enrichNewTracks(previewBeforeIds, "已导入");
}

/** 关闭预览并清理暂存:确认导入完成与取消共用 */
export async function closeImportPreview(): Promise<void> {
  for (const p of importPreview.packages) {
    try {
      await api.discardPreview(p.stagingId);
    } catch (err) {
      console.error("清理预览暂存失败", err);
    }
  }
  importPreview.open = false;
  importPreview.packages = [];
  importPreview.results = [];
}

export async function importDropped(paths: string[]): Promise<void> {
  // 音乐包(双击打开/拖入)统一走预览弹窗:用户确认后才正式导入
  if (paths.some((p) => PACKAGE_EXTS.some((e) => p.toLowerCase().endsWith(e)))) {
    await openImportPreview(paths);
    return;
  }
  const before = new Set(lib.tracks.map((t) => t.id));
  const rest = paths.filter(
    (p) => ![".tmc", ".tmcl", ".tmca"].some((e) => p.toLowerCase().endsWith(e)),
  );
  const parts: string[] = [];
  if (rest.length) {
    try {
      const r = await api.importPaths(rest);
      if (r.filesCopied) parts.push(`导入 ${r.filesCopied} 个文件`);
      if (r.transcoded) parts.push(`转码 ${r.transcoded} 个非常规格式`);
      if (r.foldersAdded) parts.push(`添加 ${r.foldersAdded} 个文件夹`);
      if (r.duplicates) parts.push(`跳过 ${r.duplicates} 个重复`);
      if (r.skipped) parts.push(`跳过 ${r.skipped} 个不支持`);
    } catch (err) {
      console.error("导入失败", err);
      parts.push("导入失败");
    }
  }
  flashStatus(parts.length ? parts.join(" · ") : "没有可导入的文件");
  await rescan();
  // 导入后自动在线补全新增曲目的封面与歌词(拖拽文件夹导入的同样生效)
  await enrichNewTracks(before, parts.length ? parts.join(" · ") : "已导入");
}

/** 从文件选择器导入 .tmc 音乐包(设置弹窗入口):与双击打开同走预览弹窗 */
export async function importTmcPick(): Promise<void> {
  const path = await api.pickTmcFile();
  if (!path) return;
  await openImportPreview([path]);
}

/** 逐张专辑在线补全,返回有更新的专辑数;并发度 3(更高会触发接口限流) */
async function enrichKeys(keys: Set<string>): Promise<number> {
  const list = [...keys];
  let enriched = 0;
  let idx = 0;
  const workers = Array.from({ length: Math.min(3, list.length) }, async () => {
    while (idx < list.length) {
      const key = list[idx++];
      try {
        const r = await api.neteaseEnrichAlbum(key);
        if (r.cover || r.lyrics > 0) enriched++;
      } catch (err) {
        console.error("在线匹配失败", err);
      }
    }
  });
  await Promise.all(workers);
  return enriched;
}

/** 找出 before 之后新增曲目中缺封面/歌词的专辑 key */
function keysOfNewTracks(before: Set<string>): Set<string> {
  const keys = new Set<string>();
  for (const t of lib.tracks) {
    if (!before.has(t.id) && (!t.cover || !t.lrcPath)) {
      keys.add(keyOf(t));
    }
  }
  return keys;
}

/** 导入通用收尾:对新增曲目自动在线匹配封面与歌词,有更新则重扫并刷新提示;quiet 时无新增不提示 */
async function enrichNewTracks(before: Set<string>, doneMsg: string, quiet = false): Promise<void> {
  const keys = keysOfNewTracks(before);
  if (keys.size === 0) {
    if (!quiet) flashStatus(doneMsg);
    return;
  }
  lib.matching = true;
  try {
    flashStatus("正在自动匹配封面与歌词…");
    const enriched = await enrichKeys(keys);
    if (enriched > 0) await rescan();
    flashStatus(enriched > 0 ? `${doneMsg} · 已为 ${enriched} 张专辑补全封面/歌词` : doneMsg);
  } finally {
    lib.matching = false;
  }
}

/** 歌曲页:为全库缺封面/歌词的专辑在线匹配(网易云公开接口) */
export async function enrichAllNetease(): Promise<void> {
  if (lib.matching) return;
  const keys = new Set<string>();
  for (const t of lib.tracks) {
    if (!t.cover || !t.lrcPath) {
      keys.add(`${t.albumArtist || t.artist}\u{1}${t.album}`);
    }
  }
  if (keys.size === 0) {
    flashStatus("所有歌曲都已有封面与歌词");
    return;
  }
  lib.matching = true;
  flashStatus(`正在在线匹配 ${keys.size} 张专辑…`);
  try {
    const done = await enrichKeys(keys);
    if (done > 0) await rescan();
    flashStatus(done > 0 ? `已为 ${done} 张专辑补全封面/歌词` : "没有找到可匹配的内容");
  } finally {
    lib.matching = false;
  }
}

/** 直接导入音乐文件:多选 → 复制进导入目录(后端已扫描并记指纹) → 自动在线匹配封面与歌词 */
export async function importMusicPick(): Promise<void> {
  const paths = await api.pickAudioFiles();
  if (!paths || paths.length === 0) return;
  const before = new Set(lib.tracks.map((t) => t.id));
  flashStatus(`正在导入 ${paths.length} 个文件…`);
  try {
    const r = await api.importPaths(paths);
    await refreshLibrary();
    if (!r.filesCopied) {
      flashStatus(r.duplicates ? `全部为重复文件,已跳过 ${r.duplicates} 个` : "没有新文件可导入");
      return;
    }
    await enrichNewTracks(before, `已导入 ${r.filesCopied} 个文件`);
  } catch (err) {
    console.error("导入音乐文件失败", err);
    flashStatus("导入失败");
  }
}

/** 单首歌的导出描述(id + 展示名) */
interface ExportItem {
  id: string;
  name: string;
}

/** 两阶段导出的阶段:packing 打包中 → ready 包裹就绪 → placing 落位中 → done 完成 */
export type ExportPhase = "packing" | "ready" | "placing" | "done";

/** 导出进度弹窗的实时状态(与设置/关于同款弹窗,由 ExportModal 渲染) */
export const exportState = reactive({
  open: false,
  phase: "packing" as ExportPhase,
  /** 来源标签(如「整张专辑」/「播放列表 · xxx」) */
  label: "",
  /** 包裹总数与已打包完成数 */
  total: 0,
  done: 0,
  /** 当前正在打包的歌名 */
  current: "",
  /** 单个包裹内部进度(TMCL 大包由后端 export-progress 事件驱动) */
  assetDone: 0,
  assetTotal: 0,
  /** 打包失败被跳过的数量 */
  packFailed: 0,
  /** 已打包好的暂存文件绝对路径 */
  files: [] as string[],
  stageDir: "",
  /** 落位结果 */
  placed: 0,
  failed: 0,
  err: "",
});

/** 统一两阶段导出:先全部打包到 Temp 暂存区(弹窗显示进度),
 *  包裹就绪后由用户点「导出」选择位置,完整文件瞬间落位,目标路径不会出现半成品 */
async function runExport(
  label: string,
  total: number,
  pack: () => Promise<string[]>,
): Promise<void> {
  exportState.open = true;
  exportState.phase = "packing";
  exportState.label = label;
  exportState.total = total;
  exportState.done = 0;
  exportState.current = "";
  exportState.assetDone = 0;
  exportState.assetTotal = 0;
  exportState.packFailed = 0;
  exportState.files = [];
  exportState.stageDir = "";
  exportState.placed = 0;
  exportState.failed = 0;
  exportState.err = "";
  try {
    exportState.stageDir = await api.makeStageDir();
    exportState.files = await pack();
    // 弹窗已被用户取消:保持关闭,不进入就绪态
    if (!exportState.open) return;
    exportState.phase = "ready";
  } catch (err) {
    console.error("导出打包失败", err);
    exportState.err = err instanceof Error ? err.message : String(err);
  }
}

/** 关闭导出弹窗:尚未导出的包裹连同暂存目录一并清理 */
export function closeExport(): void {
  if (exportState.stageDir && exportState.phase !== "done") {
    void api.cleanupStage(exportState.stageDir);
  }
  exportState.open = false;
}

/** 弹窗内点「导出」:选目标文件夹,把暂存好的完整包裹瞬间落位 */
export async function confirmExport(): Promise<void> {
  if (exportState.phase !== "ready") return;
  const dir = await api.pickExportDir();
  if (!dir) return;
  exportState.phase = "placing";
  try {
    const placed = await api.placeStaged(exportState.files, dir);
    exportState.placed = placed.length;
    exportState.failed = exportState.files.length - placed.length;
  } catch (err) {
    console.error("导出落位失败", err);
    exportState.placed = 0;
    exportState.failed = exportState.files.length;
    exportState.err = err instanceof Error ? err.message : String(err);
  }
  void api.cleanupStage(exportState.stageDir);
  exportState.phase = "done";
  const prefix = exportState.label ? `${exportState.label} · ` : "";
  flashStatus(
    exportState.failed === 0
      ? `${prefix}已导出 ${exportState.placed} 个音乐包`
      : `${prefix}已导出 ${exportState.placed}/${exportState.files.length} 个(部分失败)`,
  );
}

/** 把一批歌逐个打包进暂存区(每首一个 .tmc;单个失败跳过不中断) */
function packTmcItems(items: ExportItem[]): Promise<string[]> {
  return (async () => {
    const files: string[] = [];
    for (const it of items) {
      // 弹窗被关闭:中止后续打包,并清掉在途打包可能重建的暂存目录
      if (!exportState.open) {
        void api.cleanupStage(exportState.stageDir);
        return files;
      }
      exportState.current = it.name;
      try {
        files.push(await api.stageTmc(it.id, exportState.stageDir));
      } catch (err) {
        console.error("打包 TMC 失败", it.name, err);
        exportState.packFailed++;
      }
      exportState.done++;
    }
    if (files.length === 0) throw new Error("所有包裹打包失败");
    return files;
  })();
}

/** 单首导出入口(列表行/右键):与其他导出一样走进度弹窗 */
export async function exportTrackTmc(id: string, title: string, artist: string): Promise<void> {
  await runExport("", 1, () => packTmcItems([{ id, name: `${title} - ${artist}` }]));
}

/** 批量导出多首歌为 .tmc 音乐包(多选操作条 / 专辑导出入口) */
export async function exportTracksTmc(
  ids: string[],
  tracks: Track[],
  label?: string,
): Promise<void> {
  if (ids.length === 0) return;
  // ids 用 Set 比对:数组 includes 是 O(m),全选大曲库导出时是 n×m 级全表扫描
  const idSet = new Set(ids);
  const items = tracks
    .filter((t) => idSet.has(t.id))
    .map((t) => ({ id: t.id, name: `${t.title} - ${t.artist}` }));
  if (items.length === 0) return;
  const text = label || (items.length > 1 ? `已选 ${items.length} 首` : "");
  await runExport(text, items.length, () => packTmcItems(items));
}

/** 导出播放列表为 .tmcl:单个大包,写入进度由后端 export-progress 事件上报 */
export async function exportPlaylistTmcl(playlistId: string, name: string): Promise<void> {
  await runExport(`播放列表 · ${name}`, 1, async () => {
    exportState.current = name;
    const f = await api.stageTmcl(playlistId, exportState.stageDir);
    exportState.done = 1;
    return [f];
  });
}

/** 导出整张专辑为 .tmca 专辑包(单文件;导入只入库,专辑归类由元数据成立) */
export async function exportAlbumTmca(albumKey: string, name: string): Promise<void> {
  await runExport(`整张专辑 · ${name}`, 1, async () => {
    exportState.current = name;
    const f = await api.stageTmca(albumKey, exportState.stageDir);
    exportState.done = 1;
    return [f];
  });
}

/** 通过网易云公开接口为整张专辑在线补全封面与歌词(仅元数据,播放仍走本地文件) */
export async function enrichAlbumNetease(key: string): Promise<void> {
  flashStatus("正在在线匹配封面与歌词…");
  try {
    const r = await api.neteaseEnrichAlbum(key);
    const parts: string[] = [];
    if (r.cover) parts.push("封面已更新");
    if (r.lyrics) {
      const extras: string[] = [];
      if (r.lyricsLrclib) extras.push(`LRCLIB ${r.lyricsLrclib}`);
      if (r.lyricsKuwo) extras.push(`酷我 ${r.lyricsKuwo}`);
      parts.push(
        extras.length ? `补全 ${r.lyrics} 首歌词(${extras.join(" · ")})` : `补全 ${r.lyrics} 首歌词`,
      );
    }
    if (!parts.length) parts.push(r.skipped ? "没有找到可匹配的内容" : "元数据已是最新");
    flashStatus(parts.join(" · "));
  } catch (err) {
    console.error("在线匹配失败", err);
    flashStatus("在线匹配失败");
  }
  await rescan();
}

/** 删除曲目:音频与歌词移入应用回收站(30 天后自动清理),曲库同步移除;
 *  删除当前播放曲时清空播放器。返回是否全部删除成功(失败时调用方应留在原页面并提示)。 */
export async function deleteTracks(ids: string[]): Promise<boolean> {
  if (ids.length === 0) return false;
  let ok = false;
  try {
    const r = await api.deleteTracks(ids);
    purgeDeleted(ids);
    ok = r.deleted > 0;
    if (r.failed > 0) {
      flashStatus(
        r.deleted > 0
          ? `已删除 ${r.deleted} 首,${r.failed} 首失败(文件被占用)`
          : `删除失败:${r.failed} 首文件被占用`,
      );
    } else {
      flashStatus(r.deleted > 0 ? `已删除 ${r.deleted} 首(移入回收站,30 天内可还原)` : "删除失败");
    }
  } catch (err) {
    console.error("删除失败", err);
    flashStatus("删除失败");
  }
  await refreshLibrary();
  return ok;
}

/** 从回收站还原曲目:文件移回原位,曲库记录恢复;ids 为空还原全部 */
export async function restoreTracks(ids: string[]): Promise<void> {
  try {
    const r = await api.restoreTracks(ids);
    flashStatus(r.failed > 0 ? `已还原 ${r.restored} 首,${r.failed} 首失败` : `已还原 ${r.restored} 首`);
  } catch (err) {
    console.error("还原失败", err);
    flashStatus("还原失败");
  }
  await refreshLibrary();
}

/** 彻底删除回收站条目(不可恢复);ids 为空表示清空全部 */
export async function purgeTrash(ids: string[]): Promise<void> {
  try {
    const n = await api.purgeTrash(ids);
    flashStatus(ids.length ? `已彻底删除 ${n} 首` : `已清空回收站(共 ${n} 首)`);
  } catch (err) {
    console.error("彻底删除失败", err);
    flashStatus("操作失败");
  }
  await refreshLibrary();
}

/** 重新拉取曲库(不触发扫描) */
async function refreshLibrary(): Promise<void> {
  try {
    applyLibraryData(await api.getLibrary());
  } catch (err) {
    console.error("刷新曲库失败", err);
  }
}

export async function openImportDir(): Promise<void> {
  try {
    lib.importDir = await api.openImportDir();
  } catch (err) {
    console.error("打开导入目录失败", err);
  }
}

export async function rescan(): Promise<void> {
  if (lib.scanning) return;
  lib.scanning = true;
  lib.scanCurrent = 0;
  lib.scanTotal = 0;
  try {
    const r = await api.scan();
    // 无变化(启动扫描的常态)时跳过全量赋值,避免 computed 重建与视图重渲
    if (r.report.added || r.report.updated || r.report.removed) {
      applyLibraryData(JSON.parse(r.library) as { tracks: Track[]; folders: string[]; playlists?: Playlist[]; trash?: TrashEntry[] });
    }
  } catch (err) {
    console.error("扫描失败", err);
  } finally {
    lib.scanning = false;
  }
}

export async function addFolder(): Promise<void> {
  const path = await api.pickFolder();
  if (!path) return;
  const before = new Set(lib.tracks.map((t) => t.id));
  try {
    await api.addFolder(path);
  } catch (err) {
    console.error("添加文件夹失败", err);
    flashStatus(`添加文件夹失败: ${String(err)}`);
    return;
  }
  if (!lib.folders.includes(path)) lib.folders.push(path);
  await rescan();
  const added = lib.tracks.filter((t) => !before.has(t.id)).length;
  await enrichNewTracks(before, `已添加文件夹 · 新增 ${added} 首`);
}

export async function removeFolder(path: string): Promise<void> {
  try {
    await api.removeFolder(path);
  } catch (err) {
    console.error("移除文件夹失败", err);
    return;
  }
  lib.folders = lib.folders.filter((f) => f !== path);
  // 组件级前缀:不能字符串 starts_with,否则移除 F:\music 会误伤 F:\music2
  const prefix = path.endsWith("\\") ? path : `${path}\\`;
  lib.tracks = lib.tracks.filter((t) => t.path !== path && !t.path.startsWith(prefix));
}

// 预建的中文排序器:localeCompare 每次解析 locale 选项,预建后复用快数倍
export const zhCollator = new Intl.Collator("zh");

const keyOf = (t: Track) => `${t.albumArtist || t.artist}\u{1}${t.album}`;

/** 曲目所属专辑的唯一键(albumArtist + album) */
export const trackAlbumKey = keyOf;

export const albums = computed<Album[]>(() => {
  const map = new Map<string, Album>();
  for (const t of lib.tracks) {
    const key = keyOf(t);
    let a = map.get(key);
    if (!a) {
      a = {
        key,
        album: t.album,
        artist: t.albumArtist || t.artist,
        year: t.year,
        cover: t.cover,
        coverLarge: t.coverLarge,
        tracks: [],
      };
      map.set(key, a);
    }
    a.tracks.push(t);
    if (!a.cover && t.cover) a.cover = t.cover;
    if (!a.coverLarge && t.coverLarge) a.coverLarge = t.coverLarge;
    if (a.year == null && t.year != null) a.year = t.year;
  }
  for (const a of map.values()) {
    a.tracks.sort(
      (x, y) =>
        (x.discNo ?? 1) - (y.discNo ?? 1) ||
        (x.trackNo ?? 0) - (y.trackNo ?? 0) ||
        zhCollator.compare(x.title, y.title),
    );
  }
  return [...map.values()];
});

export const artists = computed<string[]>(() =>
  [...new Set(lib.tracks.map((t) => t.artist || "未知艺人"))].sort((a, b) =>
    zhCollator.compare(a, b),
  ),
);

export const allSongs = computed<Track[]>(() =>
  [...lib.tracks].sort((a, b) => zhCollator.compare(a.title, b.title)),
);

export function albumByKey(key: string): Album | undefined {
  return albums.value.find((a) => a.key === key);
}
