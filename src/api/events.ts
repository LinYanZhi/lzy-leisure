// ============================================================
// 休闲时光 - 事件监听封装（原 api.ts 拆分）
// 漫画/小说/视频/故事会的扫描完成等事件，浏览器 SSE / 桌面 tauri event 共用。
// ============================================================
import { onEvent } from "./transport";
import type {
  ScanDoneEvent,
  StoryclubPagesDoneEvent,
  StoryclubScanDoneEvent,
  VideoScanDoneEvent,
} from "./models";

/** 监听漫画扫描完成事件（浏览器 SSE / 桌面 tauri event 共用） */
export function onScanDone(cb: (e: ScanDoneEvent) => void): Promise<() => void> {
  return onEvent<ScanDoneEvent>("scan-done", cb);
}

/** 监听小说扫描完成事件 */
export function onNovelScanDone(
  cb: (e: { root_dir: string; novels: number; removed: number }) => void,
): Promise<() => void> {
  return onEvent("novel-scan-done", cb);
}

/** 监听视频目录扫描完成事件（添加/重扫完成后自动刷新书库） */
export function onVideoScanDone(
  cb: (e: VideoScanDoneEvent) => void,
): Promise<() => void> {
  return onEvent<VideoScanDoneEvent>("video-scan-done", cb);
}

/** 监听故事会扫描完成事件（设置/重扫根路径后自动刷新书架） */
export function onStoryclubScanDone(
  cb: (e: StoryclubScanDoneEvent) => void,
): Promise<() => void> {
  return onEvent<StoryclubScanDoneEvent>("storyclub-scan-done", cb);
}

/** 后台页数解析完成事件（解析了一批缺页数的期数，前端刷新显示真实页数） */
export function onStoryclubPagesDone(
  cb: (e: StoryclubPagesDoneEvent) => void,
): Promise<() => void> {
  return onEvent<StoryclubPagesDoneEvent>("storyclub-pages-done", cb);
}
