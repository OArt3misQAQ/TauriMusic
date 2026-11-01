/** 时间格式化(分:秒)的唯一实现:播放页/顶栏/桌面歌词/托盘面板共用。
 *  独立成模块而不是放进 player store:托盘与歌词是独立窗口,
 *  引 player 会连带实例化音频单例等副作用 */
export function fmtTime(s: number): string {
  if (!isFinite(s) || s < 0) s = 0;
  const m = Math.floor(s / 60);
  const sec = Math.floor(s % 60);
  return `${m}:${String(sec).padStart(2, "0")}`;
}
