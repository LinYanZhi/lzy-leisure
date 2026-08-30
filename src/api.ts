// ============================================================
// 休闲时光 - 命令封装（漫画 + 视频 + 小说[预留]）
//
// 按职责拆分为子模块（对外导入路径 "./api" 不变）：
//   - api/transport.ts 传输层（invoke/HTTP 调用 + 认证 + 事件 + 流 URL）
//   - api/events.ts    事件监听封装（scan-done 等）
//   - api/pickers.ts   目录/文件选择（双端）
//   - api/models.ts    数据模型
//   - api/commands.ts  api 命令集
//
// 本文件为桶文件：重导出各子模块公开面，保持与拆分前一致。
//
// 后端命令层冲突时以模块语义前缀区分：
//   - 漫画：get_comic_cover_data_url / open_comic_directory / batch_set_sort_order
//   - 视频：get_video_cover_data_url / open_video_folder / batch_set_video_sort_order
// ============================================================
export {
  isWeb,
  getToken,
  setToken,
  clearToken,
  UNAUTHORIZED_EVENT,
  checkAuth,
  login,
  videoStreamUrl,
  imgUrl,
  storyclubPdfUrl,
} from "./api/transport";
export * from "./api/models";
export { api } from "./api/commands";
export {
  onScanDone,
  onNovelScanDone,
  onVideoScanDone,
  onStoryclubScanDone,
  onStoryclubPagesDone,
} from "./api/events";
