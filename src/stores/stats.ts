import { ref, watch } from "vue";
import { api, type StatsData } from "../api";
import { player, current } from "./player";
import { ui } from "./ui";
import { StorageKeys, storageGetString, storageSet } from "./storage";

/**
 * 听歌统计(总开关见设置 → 统计与隐私):
 * - 采集只在前端(只有 WebView 知道音频是否真的在播),按「墙钟跨度」累计——
 *   窗口最小化/托盘常驻时定时器会被节流,逐秒计数会大量漏记,时间戳差不受影响
 * - 数据仅保存在本机 stats.json,冲刷走 IPC,全链路零联网
 * - 关闭开关 = 立即停表 + 丢弃未冲刷边角 + 隐藏统计页,已落盘数据保留(可在设置清除)
 */

/** 总开关(默认开启):仅存偏好,数据本体在 Rust 端 */
export const statsEnabled = ref(storageGetString(StorageKeys.stats, "1") !== "0");

/** 统计页渲染用的全量数据:打开页面时从后端拉一次 */
export const stats = ref<StatsData | null>(null);
/** 拉取失败原因(null = 未失败):给统计页重试入口,避免永久卡"正在加载" */
export const statsLoadError = ref<string | null>(null);

export function setStatsEnabled(on: boolean): void {
  if (statsEnabled.value === on) return;
  statsEnabled.value = on;
  storageSet(StorageKeys.stats, on ? "1" : "0");
  if (on) {
    beginSpanIfNeeded();
  } else {
    // 关闭即刻停表:最后一段按正常规则结算并强制冲刷(关闭前收集的数据),
    // 之后不再采集。强制冲刷不能被"开关已关"拦下,否则这批数据要等
    // 重新开启后的下一个周期才可能补发,中途退出即彻底丢失
    endSpan(true);
    void flush(true);
    // 正看着统计页时关闭:退出页面
    if (ui.view === "stats") ui.view = "albums";
  }
}

/** 清空本机全部统计数据(设置页二次确认后调用) */
export async function clearStats(): Promise<void> {
  await api.statsClear();
  stats.value = { version: 1, days: {}, tracks: {} };
}

/** 统计页打开时拉取全量数据 */
export async function loadStats(): Promise<void> {
  statsLoadError.value = null;
  try {
    stats.value = await api.getStats();
  } catch (err) {
    console.error("读取统计数据失败", err);
    statsLoadError.value = String(err);
  }
}

// ===== 采集:墙钟跨度 =====

/** 当前跨度的曲目 id:结束结算时曲目可能已切换,必须用开始时捕获的 */
let spanTrackId: string | null = null;
/** 跨度起点(Date.now 毫秒);0 = 未在计时 */
let spanStart = 0;
/** 跨度开始时捕获的曲目时长(秒),供播放次数判定 */
let spanDur = 0;
/** 本跨度已累计秒数(含已随冲刷落盘的部分):播放次数按完整会话判定 */
let sessionSec = 0;

/** 未冲刷增量 */
let pendingSec: Record<string, number> = {};
let pendingPlays: string[] = [];
let pendingDaySec = 0;

let flushTimer = 0;

/** 本地日期键(YYYY-MM-DD),冲刷归属与统计页共用同一格式 */
export function todayStr(): string {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

/** 计一次播放的阈值:听满 4 分钟,或达到曲目的至少一半(取先到者,与主流听歌平台一致) */
function countsAsPlay(sec: number, dur: number): boolean {
  return sec >= 240 || (dur > 0 && sec >= dur / 2);
}

function beginSpanIfNeeded(): void {
  const t = current.value;
  if (!statsEnabled.value || spanStart !== 0 || !t || !player.playing) return;
  spanTrackId = t.id;
  spanStart = Date.now();
  spanDur = t.duration;
}

/** 结束当前跨度并把秒数并入待冲刷缓冲;final=true 时按会话时长判定是否计一次播放 */
function endSpan(final: boolean): void {
  if (spanStart === 0 || !spanTrackId) return;
  // 单段封顶 1 小时:正常听歌每 20 秒就会被冲刷折叠一次,超长段只可能是
  // 系统睡眠时 pause 事件丢失——宁少记不多记
  const sec = Math.min((Date.now() - spanStart) / 1000, 3600);
  spanStart = 0;
  sessionSec += sec;
  pendingSec[spanTrackId] = (pendingSec[spanTrackId] ?? 0) + sec;
  pendingDaySec += sec;
  if (final && countsAsPlay(sessionSec, spanDur)) pendingPlays.push(spanTrackId);
  if (final) {
    spanTrackId = null;
    sessionSec = 0;
    spanDur = 0;
  }
}

/** 折叠当前进行中的跨度(不判定播放次数)并冲刷缓冲;force 供关闭开关时的收尾冲刷,
 *  不受"开关已关"拦截 */
async function flush(force = false): Promise<void> {
  endSpan(false);
  if (spanStart === 0 && statsEnabled.value) beginSpanIfNeeded();
  if (!statsEnabled.value && !force) return;
  if (Object.keys(pendingSec).length === 0 && pendingPlays.length === 0 && pendingDaySec <= 0) return;
  const samples = pendingSec;
  const plays = pendingPlays;
  const daySec = pendingDaySec;
  pendingSec = {};
  pendingPlays = [];
  pendingDaySec = 0;
  try {
    await api.statsAdd(samples, plays, todayStr(), daySec);
  } catch (err) {
    // 冲刷失败丢弃这批(不回填避免重复计),下一周期继续
    console.error("统计冲刷失败", err);
  }
}

function armFlush(): void {
  if (flushTimer) return;
  flushTimer = window.setInterval(() => void flush(), 20_000);
}

let statsInited = false;

/** 应用启动时接线:监听播放状态、定时冲刷、页面隐藏兜底 */
export function initStats(): void {
  if (statsInited) return; // 重挂载/HMR 时监听器不能叠加
  statsInited = true;
  watch(
    () => player.playing,
    (playing) => {
      if (playing) beginSpanIfNeeded();
      else endSpan(true);
    },
  );
  // 切曲:旧曲按完整会话结算(可能计一次播放),新曲立即开始计时
  watch(
    () => current.value?.id ?? null,
    () => {
      endSpan(true);
      beginSpanIfNeeded();
    },
  );
  armFlush();
  // 隐藏到托盘/退出前兜底冲刷,把丢失窗口压到最小
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) void flush();
  });
  window.addEventListener("beforeunload", () => void flush());
  beginSpanIfNeeded();
}
