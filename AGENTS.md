# agents.md — lzy-leisure 项目说明（给 AI 看）

> 休闲时光：本地漫画/小说/视频/故事会阅读器（Tauri 2 + React 19 + Vite + Axum HTTP 局域网服务）。
> 维护者：LinYanZhi。改动本文件前先与用户确认。

## 一、技术栈与结构

- 前端：React 19 + TypeScript + Vite + TailwindCSS（`src/`，UI 库 `@glbt/appkit-ui` 为 file: 依赖，指向 `../GLBT-gz/app-kit/shared/ui`）
- 后端：Tauri 2 (Rust) + Axum + Tokio（`src-tauri/`）；单库 SQLite（`app_data_dir/leisure.db`）
- 双端：桌面窗口（Tauri IPC）+ 浏览器/局域网（HTTP 服务，`src-tauri/src/http/`）
- 视频数据模型：`Video`（title/path/license_plate/actors[]/kinds[]/series_id/progress/year/rating/episode…）、`Actor`（stage_names/height/cup_size/birthdate/bio/images）、`Series`（剧集聚合）；`videos`/`actors`/`video_actors`/`video_tags`/`series` 表

## 二、构建（重要：有已知环境坑）

```bash
pnpm install        # ⚠️ 可能报 ERR_PNPM_IGNORED_BUILDS（lfa-ponyfill 构建脚本被忽略）；需在 pnpm-workspace.yaml 配 allowBuilds 或用 pnpm approve-builds
pnpm tauri dev      # 开发
pnpm tauri build    # 生产（NSIS 安装包）
```

⚠️ **Vite 配置加载坑（Node 22）**：`vite.config.ts` 经 `@glbt/appkit-ui/vite-config` 导入共享包，其导出指向 `node_modules` 里的 `.ts` 源码；Node 22 类型剥离禁止加载 node_modules 下 .ts，导致 `npm run build`/`npm run dev` 报 `ERR_UNSUPPORTED_NODE_MODULES_TYPE_STRIPPING`。
**可用方案**（tsx 加载器，已验证）：
```
node --import file:///<tsx-loader路径>/dist/loader.mjs node_modules/vite/bin/vite.js build
```
- tauri 构建时用 `--config <临时json>` 覆盖 `build.beforeBuildCommand` 为上述命令（不直接改 tauri.conf.json）。
- 治本方案是给 `@appkit/ui` 补构建产物（.js 入口），属于 app-kit 项目。

## 三、视频模块约定（重要）

- **视频种类 kinds**（`src/components/video/kinds.ts` ↔ 后端 `video_scanner.rs` 的 KIND_*）：`movie/short/anime/vertical/av`；AV 由扫描器命中番号/演员自动打标。
- **扫描自动识别**（`video_scanner.rs` + `commands/commands_videos.rs`）：文件名提取番号（`[A-Z]{2,6}-\d{3,5}`）、按已登记演员的 name/stage_names 匹配演员、命中即打 `av` 种类（长片 JAV 不再归 movie）。新增演员后重扫即可自动关联。
- **交互（移动优先）**：
  - 点卡片按类型分档：AV/短视频/剧集单集 → 直接播放；电影/动漫 → 详情（`VideoShelf.isPlayFirst`）
  - 继续观看行（有进度视频横排）；卡片"⋯"操作菜单（详情/播放/删除/打开目录）
  - 播放器（`PlayerView`）：双击左右 1/3 = ±10s（toast）、单击控制栏、3s 自动隐藏、屏幕常亮、移动端进入自动全屏（作用于视频容器）
  - **迷你播放器**：全屏播放点"返回"→ 收缩悬浮小窗继续播放（不杀播放）；`App.tsx` 的 `playerMinimized` 状态控制；PlayerView 用 keyed 子节点保证视频元素跨 full/mini 存活
  - 详情页：浏览/编辑两态（默认浏览态）
  - 移动端二级导航：顶部横滑 chips（`MobileVideoLayout` TOP_NAV，分类含 AV 1 键直达）
- **演员体系**：点演员 → 演员页（`ActorDetailView` 看 TA 全部影片）；「演员→导入演员」从 JSON 清单批量导入；头像存 `app_data_dir/actor_images/`。
- **后端命令**：新增命令需同步注册三处：`commands.rs`（pub use 重导出）、`lib.rs`（invoke_handler）、`http/http_dispatch.rs`（HTTP 分发）。

## 四、文档

- `docs/video-ux-redesign.md` — 视频模块交互重构设计 + 实现进度
- `docs/移动端视频应用交互设计调研报告.md` — 移动端视频 App 交互最佳实践调研
- `docs/actor-catalog.json` — 演员导入数据（示例；完整数据在用户本地 D:\视频\cat-catch\）
- `scripts/` — 维护脚本（部分含机器特定路径/个人站点细节，提交前评估敏感度）

## 五、红线

- key/token 不入 git；个人站点细节、片库数据谨慎入库。
- 涉及用户 AV 片库的内容保持中性描述；文件操作先确认。
