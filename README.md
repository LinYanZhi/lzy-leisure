# lzy-leisure (休闲时光)

**LZY 休闲时光** — 本地漫画/小说/视频阅读器 + HTTP 服务 + 局域网访问

## 功能

- 📚 **漫画阅读器** — 支持 ZIP/CBZ 压缩包、PDF、图片文件夹；自动生成封面缩略图
- 📖 **小说阅读器** — 手机端逐页图片流阅读器、点击放大/拖拽缩放、底部三按钮翻页
- 🎬 **视频播放器** — 支持视频流、字幕 (ASS/SRT)、本地媒体库管理
  - 支持**电影 / 电视剧（剧集）/ 短视频 / AV** 四类内容，多端（桌面 / 网页 / 手机）统一体验
  - **演员体系**：演员资料（身高/罩杯/生日/简介/头像）+ 点演员即看 TA 的全部影片
  - 扫描时**自动提取番号（车牌）**、**按文件名自动匹配演员**、命中番号自动打 `AV` 种类
  - 支持从 JSON 清单**批量导入演员**（示例见 `docs/actor-catalog.json`）
- 🌐 **HTTP 服务** — 基于 Axum 的静态托管 + SSE + Range 视频流，支持局域网访问
- 💾 **数据持久化** — SQLite (bundled) 存储书架、阅读进度、设置

## 技术栈

- **前端**: React 19 + TypeScript + Vite + TailwindCSS
- **后端**: Tauri 2 (Rust) + Axum + Tokio
- **UI 组件**: @glbt/appkit-ui (来自 GLBT-gz/appkit)

## 构建

```bash
# 安装依赖
pnpm install

# 开发模式
pnpm tauri dev

# 生产构建
pnpm tauri build
```

## 许可证

MIT