# TauriMusic 开发约定

Tauri 2 + Rust 后端 + Vue 3 前端的本地音乐播放器，仅 Windows 打包（CI 另在 Linux 跑 Rust 检查）。标识符 `com.art3mis.taurimusic`。前端无 vue-router，视图在 `App.vue` 用 `<component :is>` 切换；虚拟滚动用 vue-virtual-scroller。

## 检查命令（交付前必须全过）

```bash
cd src-tauri
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test

# 仓库根目录
npm run lint
npm run build   # 含 vue-tsc 类型检查
```

## 目录结构

- `src-tauri/src/lib.rs` — 全部 Tauri commands（注册于 `run()`）；托盘、单实例、文件关联、两阶段导出、在线匹配也在这里。
- `src-tauri/src/scanner.rs` — 增量扫描与元数据解析（lofty）；`model.rs` — 数据结构与 library.json 原子写；`lyrics.rs` — LRC 解析与乱码修复；`single_instance.rs` — 启动守卫（已有实例时询问重启，互斥体去重防连点叠窗）。
- `src/api.ts` — **唯一** invoke 封装层：新增后端 command 必须同步在这里加封装。
- `src/stores/` — 手写 store（shallowReactive/reactive，**不用 Pinia**）：library（曲库/扫描/导入导出/在线匹配）、player、playlists、ui（视图路由）、shortcuts、context、storage（localStorage 统一 `tm-` 前缀）。
- `scripts/check-version.mjs` — 版本一致性校验。

## 硬性规则

- **版本号三处同步**：package.json、src-tauri/tauri.conf.json、src-tauri/Cargo.toml 必须一致，CI 校验。
- **注释与 UI 文案全中文**；注释讲"为什么"（坑、平台差异、防御原因），不写"做了什么"。
- 新逻辑配单元测试：Rust 侧各文件自带 `tests` mod，断言常配中文注释。
- Rust command 里的重活一律 `spawn_blocking`。
- **README.md 是功能的单一事实来源**：新功能上线需同步更新功能列表（含数据存储位置、项目结构小节）。
- commit message 用中文短句描述行为（如「修复 xxx: 原因」）；发版推 `v*` tag，GitHub Actions release.yml 自动构建 NSIS+MSI。

## 关键数据约定（前后端必须一致）

- 专辑唯一键 = `albumArtist \u{1} album`（前端 `keyOf()`，后端 scanner）。
- Track.id = 文件路径的 64 位 hash（`scanner::hash_str`），路径变即新 id。
- 曲库持久化 `%APPDATA%\com.art3mis.taurimusic\library.json`（tmp→bak→rename 原子写，损坏回退）；封面缓存 `%LOCALAPPDATA%\...\covers\`（512px JPEG）。

## 平台与网络限制

- **在线匹配只能放 Rust 端**：CSP `connect-src` 仅允许 ipc，前端不能直接 fetch 外部 HTTP。
- 匹配链（仅补元数据，播放永远本地）：歌词 网易云→LRCLIB→酷我；封面 网易云→iTunes(cn/us)→酷我。艺人比对经 `normalize_tag`（繁→简+小写+去空白）双向包含；酷我接口返回单引号伪 JSON 需 `kuwo_fix_json` 修复，详情接口必须带 m 站 Referer。
- Linux CI 也会跑 `cargo test`：涉及路径/平台分支的逻辑要保证两端可测（用 `cfg` 拆断言，见 `is_under` 先例）。

## 仓库边界（不入库清单）

以下路径仅存本地，已列入 `.gitignore`，不要提交、也不要在文档中当作仓库内容引用：

- `web/` — 宣传页源码（本地展示用，随仓库分发无意义）。
- `.zcode/`、`.zcodeignore`、`.mimosa/` — 本地工具状态。
- `demo/`、`beyond/`、`TauriMusic-promo/` — 打包产物/含版权音频，严禁入库。
