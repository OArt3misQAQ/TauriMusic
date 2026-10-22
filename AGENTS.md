# TauriMusic 开发约定

Tauri 2 + Rust 后端 + Vue 3 前端的本地音乐播放器，仅 Windows 打包（CI 另在 Linux 跑 Rust 检查），并支持网易云免费曲目在线试听。标识符 `com.art3mis.taurimusic`。前端无 vue-router，视图在 `App.vue` 用 `<component :is>` 切换；虚拟滚动用 vue-virtual-scroller。

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
- `src-tauri/src/online.rs` — 在线音源（网易云公开搜索/取链/歌词/封面接口）；媒体 URL 域名白名单也在这里，改动前必读文件头合规边界。
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
- Track.id = 文件路径的 64 位 hash（`scanner::hash_str`），路径变即新 id；**在线曲目 id = `net:` + 网易云曲目 id**，path 为空串，不入 library.json。
- 听歌统计 `stats.json` 为 v2 结构：`days` 为本地+在线总和，`daysLocal` / `daysOnline` 分桶；在线曲目不在曲库，榜单元数据随冲刷写入 `onlineMeta`。
- 曲库持久化 `%APPDATA%\com.art3mis.taurimusic\library.json`（tmp→bak→rename 原子写，损坏回退）；封面缓存 `%LOCALAPPDATA%\...\covers\`（512px JPEG）。

## 平台与网络限制

- **在线匹配只能放 Rust 端**：CSP `connect-src` 仅允许 ipc，前端不能直接 fetch 外部 HTTP。
- 匹配链（仅补元数据，播放永远本地）：歌词 网易云→LRCLIB→酷我；封面 网易云→iTunes(cn/us)→酷我。艺人比对经 `normalize_tag`（繁→简+小写+去空白）双向包含；酷我接口返回单引号伪 JSON 需 `kuwo_fix_json` 修复，详情接口必须带 m 站 Referer。
- Linux CI 也会跑 `cargo test`：涉及路径/平台分支的逻辑要保证两端可测（用 `cfg` 拆断言，见 `is_under` 先例）。

## 在线播放链路（改前必读）

- **CSP 只放行显式白名单**：`media-src` 仅允许 `https://*.music.126.net`、`https://music.126.net`、`https://music.163.com`；`connect-src` 仍然只放 ipc——所有 HTTP 都走 Rust 端 reqwest，前端禁止直连外网。禁止改回 `media-src https:` 之类的通配放行。
- `online::restrict_media_url` 是交给 WebView 前的最后一道闸：http 统一升 https，拒绝非网易云主机、非常规端口、userinfo 与空路径；播放直链与封面图都必须经过它。曲目 id 必须是纯数字（`online::validate_id`），它还会拼进封面缓存文件名，防路径穿越。
- 直链**懒取**：进队列时不请求，真正 load 才取，取到缓存回 `track.onlineUrl`；`player.ts` 的 `loadSeq` 是竞态令牌，慢响应不得覆盖新歌。
- 在线曲目封面落 `covers/online/{id}.jpg`，前端经 `coverPathOf` 统一兜底；歌词走 `get_lyrics` 的 `net:` 分支 + 进程内缓存。
- `online.rs` 只调用网易云网页端公开接口，免费曲目直链可播；VIP/付费/受限曲目接口给空链，**不做任何绕过**，原样报错——这是合规红线。

## 仓库边界（不入库清单）

以下路径仅存本地，已列入 `.gitignore`，不要提交、也不要在文档中当作仓库内容引用：

- `web/` — 宣传页源码（本地展示用，随仓库分发无意义）。
- `.zcode/`、`.zcodeignore`、`.mimosa/` — 本地工具状态。
- `demo/`、`beyond/`、`TauriMusic-promo/` — 打包产物/含版权音频，严禁入库。
