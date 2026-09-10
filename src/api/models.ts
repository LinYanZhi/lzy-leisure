// ============================================================
// 休闲时光 - 数据模型（原 api.ts 拆分）
// 全部为纯类型定义，供 commands/transport/events 与页面组件共用。
// ============================================================

// ══════════════════════════════════════════════════════════
//  认证
// ══════════════════════════════════════════════════════════

export interface AuthStatus {
  enabled: boolean;
  authed: boolean;
}

// ══════════════════════════════════════════════════════════
//  漫画
// ══════════════════════════════════════════════════════════

export interface Comic {
  id: string;
  title: string;
  path: string;
  type: "folder" | "archive" | "series" | "pdf";
  format: string;
  page_count: number;
  size: number;
  root_dir: string;
  series_id: string;
  cover_path: string;
  reading_progress: number;
  chapter_count: number;
  sort_order: number;
  updated_at: string;
}

export interface RootDir {
  path: string;
  name: string;
  comic_count: number;
  last_scan: string;
}

export interface PageInfo {
  index: number;
  name: string;
  mime: string;
}

/** 章节页面条带索引（自定义封面裁剪预览用） */
export interface PageIndex {
  pw: number;
  pages: { w: number; h: number; ph: number }[];
  total_ph: number;
}

export interface ChapterProgress {
  id: string;
  title: string;
  page: number;
  total_pages: number;
  pct: number;
}

export interface SeriesProgress {
  series_id: string;
  chapters: ChapterProgress[];
  continue_chapter_id: string;
  all_completed: boolean;
}

export interface ScanDoneEvent {
  root_dir: string;
  comics: number;
  chapters: number;
}

// ══════════════════════════════════════════════════════════
//  视频管理
// ══════════════════════════════════════════════════════════

export interface Video {
  id: string;
  title: string;
  path: string;
  description: string;
  license_plate: string;
  cover: string;
  duration: string;
  file_size: string;
  file_type: string;
  fps: number | null;
  frame_width: number | null;
  frame_height: number | null;
  sort_order: number;
  actors: string[];
  tags: string[];
  /** 种类（电影/短视频/动漫/竖屏，可多选） */
  kinds: string[];
  /** 同目录完全同名（仅扩展名不同）的字幕文件路径；空表示无字幕 */
  subtitle_path: string;
  /** 归属的导入路径（源）；扫描时填充，用于按源过滤与整源移除 */
  root_dir: string;
  /** 所属剧集（series.id）；空串表示不属于任何剧集 */
  series_id: string;
  /** 播放进度（秒），用于续播；0 表示未观看 */
  progress: number;
  /** 年份（如 "2020"）；文件名解析填充，空串表示未知 */
  year: string;
  /** 评分（0-10）；0 表示未评分 */
  rating: number;
  /** 集数标识（如 "第01话" / "S01E05"）；文件名解析填充，空串表示无集数 */
  episode: string;
  created_at: string;
  updated_at: string;
}

/** 视频剧集（Series）：多个视频聚合为一部剧/一个合集 */
export interface VideoSeries {
  id: string;
  title: string;
  cover: string;
  description: string;
  video_count: number;
  /** 剧集封面候选：剧集内最早登记的视频 id（前端用其封面图展示剧集卡） */
  cover_video_id: string;
  sort_order: number;
  created_at: string;
  updated_at: string;
}

/** 剧集详情：剧集信息 + 其下视频（按标题自然排序） */
export interface SeriesDetail {
  series: VideoSeries;
  videos: Video[];
}

/** 视频导入路径（源）记录 */
export interface VideoRootDir {
  path: string;
  name: string;
  video_count: number;
  last_scan: string;
}

export interface Actor {
  id: string;
  name: string;
  stage_names: string[];
  height: string;
  cup_size: string;
  birthdate: string;
  bio: string;
  images: string[];
  sort_order: number;
  created_at: string;
  updated_at: string;
}

/** 演员 + 名下视频数（演员浏览卡片用） */
export interface ActorWithCount extends Actor {
  video_count: number;
}

export interface TagGroup {
  id: string;
  name: string;
  sort_order: number;
  created_at: string;
}

export interface Tag {
  id: string;
  name: string;
  color: string;
  group_id: string;
  sort_order: number;
  created_at: string;
}

export interface ScanResult {
  total: number;
  new_count: number;
  duplicate_count: number;
  new_videos: Video[];
}

export interface VideoQuery {
  title?: string;
  tag_ids?: string[];
  match_all?: boolean;
  actor_ids?: string[];
  vertical_only?: boolean;
  /** 种类过滤（任一匹配） */
  kinds?: string[];
  /** 按导入路径（源）过滤；缺省表示全部 */
  root_dir?: string;
  /** 按剧集过滤；缺省表示全部，"unassigned" 表示未归入任何剧集 */
  series_id?: string;
}

export interface VideoEdit {
  id: string;
  title: string;
  description: string;
  license_plate: string;
  /** 年份（如 "2020"） */
  year: string;
  /** 评分（0-10；0 表示未评分） */
  rating: number;
  kinds: string[];
  actor_ids: string[];
  tag_ids: string[];
}

export interface ActorInput {
  id?: string;
  name: string;
  stage_names: string[];
  height: string;
  cup_size: string;
  birthdate: string;
  bio: string;
  sort_order?: number;
}

export interface TagInput {
  id?: string;
  name: string;
  color: string;
  group_id: string;
  sort_order?: number;
}

/** 监听视频目录扫描完成事件（添加/重扫完成后自动刷新书库） */
export interface VideoScanDoneEvent {
  root_dir: string;
  new_count: number;
  duplicate_count: number;
  removed?: number;
}

// ══════════════════════════════════════════════════════════
//  小说（EPUB：单本即一部书，只读解析）
// ══════════════════════════════════════════════════════════

/** 章节（id 即 spine 索引） */
export interface NovelChapter {
  id: number;
  title: string;
  href: string;
}

export interface Novel {
  id: string;
  title: string;
  author: string;
  path: string;
  /** 封面缓存相对路径（应用数据目录），空表示尚未提取 */
  cover_path: string;
  /** 阅读进度：章节内滚动位置（字符偏移），0 表示未读 */
  reading_pos: number;
  /** 当前章节（spine 索引，0 起；空串表示未读） */
  chapter: string;
  /** 归属导入源；单文件导入为空串 */
  root_dir: string;
  chapter_count: number;
  sort_order: number;
  created_at: string;
  updated_at: string;
  chapters: NovelChapter[];
}

/** 小说导入路径（源）记录 */
export interface NovelRootDir {
  path: string;
  name: string;
  novel_count: number;
  last_scan: string;
}

// ══════════════════════════════════════════════════════════
//  故事会
// ══════════════════════════════════════════════════════════

/** 故事会期数记录（根目录下每个子目录 = 一年，内含 PDF 期数） */
export interface StoryIssue {
  id: string;
  title: string;
  /** 所属年份目录名（根目录直接放 PDF 时为空串） */
  year: string;
  path: string;
  page_count: number;
  size: number;
  /** 阅读进度：当前跨页的第一页页码（0 起） */
  reading_progress: number;
  sort_order: number;
  updated_at: string;
}

/** 监听故事会扫描完成事件（设置/重扫根路径后自动刷新书架） */
export interface StoryclubScanDoneEvent {
  root_dir: string;
  issues: number;
  years: number;
}

/** 后台页数解析完成事件（解析了一批缺页数的期数，前端刷新显示真实页数） */
export interface StoryclubPagesDoneEvent {
  parsed: number;
}
