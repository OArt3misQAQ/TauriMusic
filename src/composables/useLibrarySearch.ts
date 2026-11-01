import { ref, watch, onUnmounted } from "vue";
import { ui, type ViewName } from "../stores/ui";

/** 曲库搜索框共通行为:180ms 防抖写回 ui.search + 搜索视图进出导航。
 *  Sidebar 与 MobileLibraryBar 两处逐字重复的实现收敛于此;
 *  卸载时清定时器,视图切换/HMR 不泄漏。defaultView 是清空搜索后的回退视图 */
export function useLibrarySearch(defaultView: ViewName) {
  const searchInput = ref(ui.search);
  /** 搜索前的视图:清空搜索时回到原处而不是固定落回默认页 */
  const viewBeforeSearch = ref<ViewName>(defaultView);
  let timer = 0;

  watch(searchInput, (v) => {
    window.clearTimeout(timer);
    timer = window.setTimeout(() => {
      if (v.trim()) {
        if (ui.view !== "search") viewBeforeSearch.value = ui.view;
        ui.view = "search";
      } else if (ui.view === "search") {
        ui.view = viewBeforeSearch.value;
      }
      ui.search = v;
    }, 180);
  });

  // 打开专辑/艺人等入口会清空 ui.search,同步回输入框
  watch(
    () => ui.search,
    (v) => {
      if (v === "" && searchInput.value !== "") searchInput.value = "";
    },
  );

  onUnmounted(() => window.clearTimeout(timer));

  return { searchInput, viewBeforeSearch };
}
