import { createApp } from "vue";
import App from "./App.vue";
import "./style.css";
import "vue-virtual-scroller/dist/vue-virtual-scroller.css";
import { closeCtx } from "./stores/context";
import { isMobile } from "./platform";

// 手机端布局用 html.mobile 切换,而不是宽度断点:横屏宽度超过 860 时
// 仍应保留底部 Tab 与迷你播放条,不能因为变宽就退回桌面双栏壳层
if (isMobile) document.documentElement.classList.add("mobile");

createApp(App).mount("#app");

// 屏蔽 WebView2 的浏览器右键菜单(另存为/刷新/更多工具等);
// 文本输入框保留系统的剪切/复制/粘贴菜单;空白区域右键同时收起自定义菜单
document.addEventListener("contextmenu", (e) => {
  const t = e.target as HTMLElement | null;
  if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable)) return;
  e.preventDefault();
  closeCtx();
});
