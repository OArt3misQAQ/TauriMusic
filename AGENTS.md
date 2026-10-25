# TauriMusic 开发约定

Tauri 2 + Rust 后端 + Vue 3 前端的本地音乐播放器，桌面端仅 Windows 打包（CI 另在 Linux 跑 Rust 检查），并支持网易云 / QQ 音乐 / 酷狗音乐 / 酷我音乐免费曲目在线试听。另有 Android 版：默认在线音乐、不提供主动导入，本地曲目通过系统「分享」/「用 TauriMusic 打开」进入。标识符 `com.art3mis.taurimusic`。前端无 vue-router，视图在 `App.vue` 用 `<component :is>` 切换；虚拟滚动用 vue-virtual-scroller。

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
- `src-tauri/src/online.rs` — 在线音源（网易云 / QQ / 酷狗 / 酷我公开搜索、取链、歌词、封面接口）；媒体 URL 域名白名单与酷我免费白名单也在这里，改动前必读文件头合规边界。
- `src/api.ts` — **唯一** invoke 封装层：新增后端 command 必须同步在这里加封装。
- `src/platform.ts` — 运行平台判定；Android 默认进在线音乐、隐藏桌面专属入口。
- `src/stores/` — 手写 store（shallowReactive/reactive，**不用 Pinia**）：library（曲库/扫描/导入导出/在线匹配）、player、playlists、ui（视图路由）、shortcuts、context、storage（localStorage 统一 `tm-` 前缀）。
- `src-tauri/gen/android/app/src/main/java/.../MainActivity.kt` — Android 专属入口：系统分享 Intent 接收、返回键转前端、状态栏/手势条安全区注入、系统栏图标明暗。
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
- Track.id = 文件路径的 64 位 hash（`scanner::hash_str`），路径变即新 id；**在线曲目 id = 来源前缀 + 平台 id**（`net:` 网易云 / `qq:` songmid / `kugou:` hash / `kuwo:` rid），path 为空串，不入 library.json。
- 听歌统计 `stats.json` 为 v2 结构：`days` 为本地+在线总和，`daysLocal` / `daysOnline` 分桶；在线曲目不在曲库，榜单元数据随冲刷写入 `onlineMeta`。
- 曲库持久化 `%APPDATA%\com.art3mis.taurimusic\library.json`（tmp→bak→rename 原子写，损坏回退）；封面缓存 `%LOCALAPPDATA%\...\covers\`（512px JPEG）。

## 平台与网络限制

- **在线匹配只能放 Rust 端**：CSP `connect-src` 仅允许 ipc，前端不能直接 fetch 外部 HTTP。
- 匹配链（仅补元数据，播放永远本地）：歌词 网易云→LRCLIB→酷我；封面 网易云→iTunes(cn/us)→酷我。艺人比对经 `normalize_tag`（繁→简+小写+去空白）双向包含；酷我接口返回单引号伪 JSON 需 `kuwo_fix_json` 修复，详情接口必须带 m 站 Referer。
- Linux CI 也会跑 `cargo test`：涉及路径/平台分支的逻辑要保证两端可测（用 `cfg` 拆断言，见 `is_under` 先例）。

## 移动端约定（Android）

- 桌面专属能力（托盘、单实例、SMTC、rfd 文件对话框、回收站、注册表）在 Rust 侧按 `cfg(desktop)` / 目标 OS 门控，前端入口按 `isMobile` 隐藏；不要用运行时 UA 判定后端能力。
- **Android 不提供主动导入**：本地曲目只从系统分享 / 「用 TauriMusic 打开」进入。`MainActivity` 把 `content://` 音频流复制到 `cache/shared-inbox`（保留原文件名，同名加序号），每首写一份同名 `.json` 说明；Rust `take_shared_audio` 读一份删一份，前端轮询后走 `import_paths` 入库并直接播放。
- 分享收件箱是"一音频一说明"，前后端都不维护共享队列文件；新增处理逻辑不要改回单文件队列（读写会互相覆盖）。
- 安全区由 `MainActivity` 把系统栏高度注入 `--tm-safe-top` / `--tm-safe-bottom`，CSS 必须用这两个变量；Android WebView 的 `env(safe-area-inset-*)` 不覆盖状态栏与手势条。
- 返回键先调 `window.__tmBack`（App.vue 注册）收播放页/弹窗/底部操作卡片，返回 `false` 才退出 Activity。
- 手机壳层是独立布局：`MobileTabBar`（在线 / 资料库 / 我的）+ `MobilePlayerBar`（迷你播放条）+ `MobileLibraryBar`（资料库搜索与分类），用 `html.mobile` 切换而不是宽度断点——横屏变宽也不能退回桌面双栏。桌面 `TopBar` / `Sidebar` 只在 `!isMobile` 时渲染。
- 触屏没有 hover：行内操作必须常驻或收进 `⋯` 菜单（`TrackRow` / `PlaylistDetail` / 在线保存按钮已按此处理），不要只靠 `:hover` 显形。
- 手机性能模式 `ui.perfMode`（localStorage `tm-perf`）默认 `smooth`：会在 `<html>` 挂 `perf-smooth`，全局关掉 ambient 与所有 backdrop-filter、播放页改静态深色底；`rich` 才保留完整玻璃/氛围。新增手机特效必须同时考虑这两档，不能默认走重滤镜。
- 应用图标源稿是 `design/icon.svg`；改完执行 `npx tauri icon design/icon.svg`，它会同时更新 `src-tauri/icons` 与 Android 工程的 mipmap。Android 自适应图标底色在 `gen/android/.../values/ic_launcher_background.xml`（墨底），冷启动窗口在 `drawable/launch_background.xml` 与 themes。
- 分享进来的音频只接受扫描器直放格式（mp3 / m4a / flac / ogg / oga / opus / wav）：移动端没有 ffmpeg，其它格式入库也扫不到。
- **前端资源在编译期嵌入 `libtauri_music_lib.so`**（不读 APK 的 `assets/`）：改完前端必须重编 Android 目标；只替换 `gen/android/app/src/main/assets/` 会让页面停在旧 bundle。Windows 未开开发者模式时的完整打包绕法见 README「构建」小节。

## 在线播放链路（改前必读）

- **CSP 只放行显式白名单**：`media-src` 仅允许 `https://*.music.126.net`、`https://music.126.net`、`https://music.163.com`、`https://*.stream.qqmusic.qq.com`、`https://*.kugou.com`、`https://kugou.com`、`https://*.kuwo.cn`、`https://kuwo.cn`；`connect-src` 仍然只放 ipc——所有 HTTP 都走 Rust 端 reqwest，前端禁止直连外网。禁止改回 `media-src https:` 之类的通配放行。
- `online::restrict_media_url` / `restrict_cover_url`（共用 `restrict_url`）是交给 WebView 前的最后一道闸：http 统一升 https，按来源拒绝非官方主机、非常规端口、userinfo 与空路径；播放直链与封面图都必须经过它。曲目 id 必须符合来源形状（`online::validate_id`：数字 / songmid / 32 位 hex），它还会拼进封面缓存文件名，防路径穿越。
- 直链**懒取**：进队列时不请求，真正 load 才取，取到缓存回 `track.onlineUrl`；`player.ts` 的 `loadSeq` 是竞态令牌，慢响应不得覆盖新歌。
- **音质固定取免费上限，无选择 UI**：`play_url(source, id)` 不收档位参数，恒取各平台给匿名听众的最高免费档——网易云 `br=320000`（无 320k 源时接口自己降级）、酷狗优先 `HQFileHash`（320k，档位由 hash 决定而非 `br` 参数，搜索时按 `HQPrivilege<10` 记进 `KUGOU_HQ_HASHES` 会话表）；QQ 与酷我匿名最高 128k，照原样取。酷狗 HQ 被接口拒绝时回落 128k，高音质是上限不是硬性条件。
- **保存到本地库**：`online_download` = 取链（与播放同一道闸）→ `download_client`（300s 超时、无 Cookie/Referer）→ 按直链推容器扩展名 → `write_audio_tags` 写标签+内嵌封面、`lyrics::render` 回写同名 `.lrc` → 复用 `import_paths_blocking` 落导入目录并 SHA 判重。标签/歌词写入失败都不致命，仅元数据降级。
- 在线曲目封面落 `covers/online/{source}_{id}.jpg`（网易云沿用 `{id}.jpg`），前端经 `coverPathOf` 统一兜底；歌词走 `get_lyrics` 的来源前缀分发 + 进程内缓存。
- 付费红线：网易云/QQ/酷狗由接口自身对 VIP 返回空链；**酷我 anti.s 对 VIP 也返回直链**，因此 `online_play_url` 只允许本次会话搜索结果里 `PAY=0` 的酷我曲目取链（`KUWO_FREE_IDS` 白名单）。任何情况下都不做登录态、签名绕过或付费解锁。

## 仓库边界（不入库清单）

以下路径仅存本地，已列入 `.gitignore`，不要提交、也不要在文档中当作仓库内容引用：

- `web/` — 宣传页源码（本地展示用，随仓库分发无意义）。
- `.zcode/`、`.zcodeignore`、`.mimosa/` — 本地工具状态。
- `demo/`、`beyond/`、`TauriMusic-promo/` — 打包产物/含版权音频，严禁入库。
