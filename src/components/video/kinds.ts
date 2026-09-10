/**
 * 视频种类定义（与后端 video_scanner.rs 的 KIND_* 常量对应）
 * kinds 可多选，值存 videos.kinds JSON 数组。
 */
export const VIDEO_KINDS: { id: string; label: string }[] = [
  { id: "movie", label: "电影" },
  { id: "short", label: "短视频" },
  { id: "anime", label: "动漫" },
  { id: "vertical", label: "竖屏" },
  { id: "av", label: "AV" },
];

export const KIND_LABEL: Record<string, string> = Object.fromEntries(
  VIDEO_KINDS.map((k) => [k.id, k.label]),
);
