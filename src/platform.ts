/**
 * 运行平台判定:Tauri 移动壳(Android WebView)的 UA 必带 "Android";
 * 桌面 WebView2 与桌面浏览器不会出现。多窗口(桌面歌词/托盘面板)、
 * 文件关联、原生目录选择等桌面专属设施按此隐藏入口,避免点击后报错。
 * 注意不能只判 "Mobile":部分安卓平板 UA 不含该词
 */
export const isMobile = /Android|iPhone|iPad/i.test(navigator.userAgent);
