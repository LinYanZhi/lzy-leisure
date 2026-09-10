// ============================================================
// 休闲时光 - api 命令集（原 api.ts 拆分）
// 按业务域分组的统一命令调用；对外经 api.ts 桶文件重导出。
// ============================================================
import { call, imgUrl, isWeb } from "./transport";
import { pickDirectory, pickFile, pickNovelFile } from "./pickers";
import type {
  Actor,
  ActorInput,
  ActorWithCount,
  Comic,
  Novel,
  NovelRootDir,
  PageIndex,
  PageInfo,
  RootDir,
  ScanResult,
  SeriesDetail,
  SeriesProgress,
  StoryIssue,
  Tag,
  TagGroup,
  TagInput,
  Video,
  VideoEdit,
  VideoQuery,
  VideoRootDir,
  VideoSeries,
} from "./models";

// ══════════════════════════════════════════════════════════
//  api 命令集
// ══════════════════════════════════════════════════════════

export const api = {
  // ════════════ 漫画：根目录 ════════════
  addRootDir: (path: string) => call<string>("add_root_dir", { path }),
  removeRootDir: (path: string) => call<string>("remove_root_dir", { path }),
  getRootDirs: () => call<RootDir[]>("get_root_dirs"),
  scanRootDir: (path: string) => call<string>("scan_root_dir", { path }),
  /** 以漫画为基本单位重新扫描（仅更新该漫画，不动同根其他漫画） */
  rescanComic: (comicId: string) => call<string>("rescan_comic", { comic_id: comicId }),
  rescanAll: () => call<string>("rescan_all"),
  clearCoverCache: () => call<string>("clear_cover_cache"),
  pickComicDir: () => pickDirectory("选择漫画根目录"),

  // ════════════ 漫画：查询与编辑 ════════════
  getTopLevelComics: () => call<Comic[]>("get_top_level_comics"),
  getChapters: (seriesId: string) => call<Comic[]>("get_chapters", { series_id: seriesId }),
  getComic: (comicId: string) => call<Comic | null>("get_comic", { comic_id: comicId }),
  deleteComic: (comicId: string) => call<string>("delete_comic", { comic_id: comicId }),
  renameComic: (comicId: string, title: string) =>
    call<string>("rename_comic", { comic_id: comicId, title }),
  openComicDirectory: (comicId: string) =>
    call<string>("open_comic_directory", { comic_id: comicId }),
  /** 批量设置排序（漫画书架拖拽排序后保存） */
  batchSetSortOrder: (orders: [number, string][]) =>
    call<void>("batch_set_sort_order", { orders }),

  // ════════════ 漫画：进度 & 完结 ════════════
  getSeriesProgress: (seriesId: string) =>
    call<SeriesProgress>("get_series_progress", { series_id: seriesId }),
  getAllSeriesProgress: () => call<Record<string, SeriesProgress>>("get_all_series_progress"),
  getCompletedStatus: (comicId: string) =>
    call<boolean | null>("get_completed_status", { comic_id: comicId }),
  setCompletedStatus: (comicId: string, completed: boolean) =>
    call<string>("set_completed_status", { comic_id: comicId, completed }),
  getAllCompletedStatus: () => call<Record<string, boolean>>("get_all_completed_status"),
  syncIndex: (comicId: string) => call<boolean>("sync_index", { comic_id: comicId }),

  // ════════════ 漫画：封面 & 页面 ════════════
  /** 封面（浏览器模式返回二进制流 URL，桌面返回 base64 data URL） */
  getComicCoverDataUrl: (comicId: string) =>
    isWeb
      ? Promise.resolve(imgUrl(["comic-cover", comicId]))
      : call<string>("get_comic_cover_data_url", { comic_id: comicId }),
  listPages: (comicId: string) => call<PageInfo[]>("list_pages", { comic_id: comicId }),
  /** 页面（浏览器模式返回二进制流 URL，桌面返回 base64 data URL） */
  getPageDataUrl: (comicId: string, pageIndex: number) =>
    isWeb
      ? Promise.resolve(imgUrl(["comic-page", comicId, String(pageIndex)]))
      : call<string>("get_page_data_url", { comic_id: comicId, page_index: pageIndex }),
  /** 章节页面条带索引（自定义封面裁剪预览） */
  getComicPageIndex: (comicId: string) =>
    call<PageIndex>("get_comic_page_index", { comic_id: comicId }),
  /** 页面预览缩略图（400 宽缩放，拼接条带用；浏览器模式返回二进制流 URL） */
  getPagePreviewDataUrl: (comicId: string, pageIndex: number) =>
    isWeb
      ? Promise.resolve(imgUrl(["comic-preview", comicId, String(pageIndex)]))
      : call<string>("get_page_preview_data_url", { comic_id: comicId, page_index: pageIndex }),
  /** 自定义章节封面：从章节拼接条带按像素偏移截取 5:7 窗口生成 */
  setComicCoverFromOffset: (comicId: string, offset: number) =>
    call<void>("set_comic_cover_from_offset", { comic_id: comicId, offset }),
  /** 恢复章节封面为自动生成 */
  resetComicCover: (comicId: string) => call<void>("reset_comic_cover", { comic_id: comicId }),
  /** 读取 PDF 章节（浏览器模式返回二进制流 URL，pdf.js 按 Range 拉取；桌面返回 base64） */
  getComicPdfData: (comicId: string) =>
    isWeb
      ? Promise.resolve(imgUrl(["comic-pdf", comicId]))
      : call<string>("get_comic_pdf_data", { comic_id: comicId }),
  /** 上传封面（overwrite=true 覆盖已有封面，用于 PDF 手动截取；默认仅首次生效） */
  uploadComicCover: (comicId: string, dataUrl: string, overwrite?: boolean) =>
    call<string>("upload_comic_cover", {
      comic_id: comicId,
      data_url_value: dataUrl,
      overwrite: overwrite ?? false,
    }),

  // ════════════ 漫画：阅读进度 & 配置 ════════════
  setReadingProgress: (comicId: string, page: number) =>
    call<string>("set_reading_progress", { comic_id: comicId, page }),
  getReadingProgress: (comicId: string) =>
    call<number>("get_reading_progress", { comic_id: comicId }),
  getComicConfig: (comicId: string) =>
    call<Record<string, string>>("get_comic_config", { comic_id: comicId }),
  setComicConfig: (comicId: string, config: Record<string, string>) =>
    call<string>("set_comic_config", { comic_id: comicId, config }),

  // ════════════ 视频：扫描 / 添加 ════════════
  scanDirectory: (dir: string) => call<ScanResult>("scan_directory", { dir }),
  addVideo: (path: string) => call<Video>("add_video", { path }),
  pickVideoDir: () => pickDirectory("选择视频目录"),
  pickVideoFile: () => pickFile("选择视频文件"),

  // ════════════ 视频：查询与编辑 ════════════
  listVideos: (query?: VideoQuery) => call<Video[]>("list_videos", { query: query ?? null }),
  /** 全部视频导入路径（源） */
  getVideoRoots: () => call<VideoRootDir[]>("get_video_roots", {}),
  /** 添加视频导入路径并扫描该目录（含子目录）下全部视频 */
  addVideoRoot: (path: string) => call<string>("add_video_root", { path }),
  /** 移除视频导入路径（连同其下视频记录，不删本地文件） */
  removeVideoRoot: (path: string) => call<null>("remove_video_root", { path }),
  /** 重新扫描单个视频导入路径（更新元数据/字幕，移除已不存在的记录） */
  rescanVideoRoot: (path: string) => call<string>("rescan_video_root", { path }),
  getVideo: (videoId: string) => call<Video | null>("get_video", { video_id: videoId }),
  /** 读取视频同名字幕内容（UTF-8；ass/srt 常见 GBK 自动转码） */
  getVideoSubtitle: (videoId: string) => call<string>("get_video_subtitle", { video_id: videoId }),
  updateVideo: (edit: VideoEdit) => call<string>("update_video", { edit }),
  /** 前端按需补全媒体元数据（时长/分辨率；大小与格式由后端读取） */
  updateVideoMediaMeta: (
    videoId: string,
    duration: string,
    frameWidth: number | null,
    frameHeight: number | null,
  ) =>
    call<null>("update_video_media_meta", {
      video_id: videoId,
      duration,
      frame_width: frameWidth,
      frame_height: frameHeight,
    }),
  deleteVideo: (videoId: string) => call<string>("delete_video", { video_id: videoId }),
  getVideoPlayPath: (videoId: string) =>
    call<string>("get_video_play_path", { video_id: videoId }),
  openVideoFolder: (videoId: string) => call<string>("open_video_folder", { video_id: videoId }),

  // ════════════ 视频：剧集（Series） ════════════
  listSeries: () => call<VideoSeries[]>("list_series", {}),
  getSeries: (seriesId: string) => call<SeriesDetail>("get_series", { series_id: seriesId }),
  createSeries: (title: string, description?: string) =>
    call<VideoSeries>("create_series", { title, description: description ?? "" }),
  /** 目录一键成剧集：该目录（含子目录）下全部视频登记并归入，目录名作标题 */
  createSeriesFromDir: (path: string) => call<VideoSeries>("create_series_from_dir", { path }),
  updateSeries: (seriesId: string, title: string, description: string) =>
    call<null>("update_series", { series_id: seriesId, title, description }),
  deleteSeries: (seriesId: string) => call<null>("delete_series", { series_id: seriesId }),
  /** 批量设置视频归属；seriesId 传空串表示解除归属 */
  setVideosSeries: (seriesId: string, videoIds: string[]) =>
    call<null>("set_videos_series", { series_id: seriesId, video_ids: videoIds }),
  /** 保存播放进度（秒） */
  updateVideoProgress: (videoId: string, seconds: number) =>
    call<null>("update_video_progress", { video_id: videoId, seconds }),

  // ════════════ 视频：封面 ════════════
  /** 封面（浏览器模式返回二进制流 URL，桌面返回 base64 data URL） */
  getVideoCoverDataUrl: (videoId: string) =>
    isWeb
      ? Promise.resolve(imgUrl(["video-cover", videoId]))
      : call<string>("get_video_cover_data_url", { video_id: videoId }),
  /** 是否已有可用封面（无则前端 JS 截帧生成） */
  hasVideoCover: (videoId: string) => call<boolean>("has_video_cover", { video_id: videoId }),
  uploadCover: (videoId: string, dataUrl: string) =>
    call<string>("upload_cover", { video_id: videoId, data_url_value: dataUrl }),

  // ════════════ 视频：演员 ════════════
  listActors: () => call<Actor[]>("list_actors"),
  /** 演员 + 名下视频数（演员浏览卡片用） */
  listActorsWithCounts: () => call<ActorWithCount[]>("list_actors_with_counts"),
  /** 批量导入演员（JSON 清单；按 name 幂等，重复导入安全） */
  importActorsBatch: (actors: ActorInput[]) => call<number>("import_actors_batch", { actors }),
  saveActor: (input: ActorInput) => call<Actor>("save_actor", { input }),
  deleteActor: (actorId: string) => call<string>("delete_actor", { actor_id: actorId }),
  uploadActorImage: (actorId: string, dataUrl: string) =>
    call<string>("upload_actor_image", { actor_id: actorId, data_url_value: dataUrl }),
  /** 头像（浏览器模式返回二进制流 URL，桌面返回 base64 data URL） */
  getActorImageDataUrl: (actorId: string) =>
    isWeb
      ? Promise.resolve(imgUrl(["actor", actorId]))
      : call<string>("get_actor_image_data_url", { actor_id: actorId }),

  // ════════════ 视频：标签组 / 标签 ════════════
  listTagGroups: () => call<TagGroup[]>("list_tag_groups"),
  saveTagGroup: (name: string, id?: string) =>
    call<TagGroup>("save_tag_group", { name, id: id ?? null }),
  deleteTagGroup: (groupId: string) => call<string>("delete_tag_group", { group_id: groupId }),
  listTags: () => call<Tag[]>("list_tags"),
  saveTag: (input: TagInput) => call<Tag>("save_tag", { input }),
  deleteTag: (tagId: string) => call<string>("delete_tag", { tag_id: tagId }),

  // ════════════ 小说 ════════════
  listNovels: () => call<Novel[]>("list_novels"),
  getNovel: (novelId: string) => call<Novel | null>("get_novel", { novel_id: novelId }),
  /** 单文件导入 EPUB（幂等：已导入直接返回） */
  addNovel: (path: string) => call<string>("add_novel", { path }),
  deleteNovel: (novelId: string) => call<string>("delete_novel", { novel_id: novelId }),
  /** 改名（只改库记录，不碰文件） */
  renameNovel: (novelId: string, title: string) =>
    call<string>("rename_novel", { novel_id: novelId, title }),
  openNovelFolder: (novelId: string) =>
    call<string>("open_novel_folder", { novel_id: novelId }),
  pickNovelDir: () => pickDirectory("选择小说目录"),
  pickNovelFile: () => pickNovelFile("选择 EPUB / TXT 文件"),

  // ════════════ 小说：导入路径（源） ════════════
  getNovelRoots: () => call<NovelRootDir[]>("get_novel_roots"),
  /** 添加小说目录并扫描（含子目录）下全部 EPUB */
  addNovelRoot: (path: string) => call<string>("add_novel_root", { path }),
  /** 移除小说目录（连同其下书籍记录，不删本地文件） */
  removeNovelRoot: (path: string) => call<string>("remove_novel_root", { path }),
  /** 重新扫描单个小说目录（登记新增，移除已不存在的记录） */
  rescanNovelRoot: (path: string) => call<string>("rescan_novel_root", { path }),

  // ════════════ 小说：阅读 ════════════
  /** 封面（浏览器模式返回二进制流 URL，桌面返回 base64 data URL） */
  getNovelCoverDataUrl: (novelId: string) =>
    isWeb
      ? Promise.resolve(imgUrl(["novel-cover", novelId]))
      : call<string>("get_novel_cover_data_url", { novel_id: novelId }),
  /** 读取章节纯文本（只读原文件） */
  getNovelChapterContent: (novelId: string, chapterIndex: number) =>
    call<string>("get_novel_chapter_content", { novel_id: novelId, chapter_index: chapterIndex }),
  /** 保存阅读进度（chapter 为章节索引，pos 为章节内字符偏移） */
  setNovelProgress: (novelId: string, chapter: number, pos: number) =>
    call<string>("set_novel_progress", { novel_id: novelId, chapter, pos }),

  // ════════════ 故事会：根路径 & 扫描 ════════════
  pickStoryDir: () => pickDirectory("选择故事会目录"),
  storyclubSetRoot: (path: string) => call<string>("storyclub_set_root", { path }),
  storyclubGetRoot: () => call<string | null>("storyclub_get_root"),
  storyclubRemoveRoot: () => call<null>("storyclub_remove_root"),
  storyclubRescan: () => call<string>("storyclub_rescan"),

  // ════════════ 故事会：期数 & 阅读 ════════════
  storyclubListIssues: () => call<StoryIssue[]>("storyclub_list_issues"),
  /** 期数 PDF 字节（base64；阅读器回退方案） */
  storyclubGetPdfData: (issueId: string) =>
    call<string>("storyclub_get_pdf_data", { issue_id: issueId }),
  /** 期数 PDF 磁盘路径（桌面端 asset 协议流式加载） */
  storyclubGetPdfPath: (issueId: string) =>
    call<string>("storyclub_get_pdf_path", { issue_id: issueId }),
  /** 保存阅读进度（page = 当前跨页的第一页页码，0 起） */
  storyclubSetProgress: (issueId: string, page: number) =>
    call<null>("storyclub_set_progress", { issue_id: issueId, page }),
  /** 回写真实页数（打开阅读器时由 pdf.js 解析，与后台解析幂等） */
  storyclubUpdatePageCount: (issueId: string, pageCount: number) =>
    call<null>("storyclub_update_page_count", { issue_id: issueId, page_count: pageCount }),
  storyclubOpenFolder: (issueId: string) =>
    call<string>("storyclub_open_folder", { issue_id: issueId }),
  /** 打开年份所在目录（资源管理器选中该年份目录，不进入） */
  storyclubOpenYearFolder: (year: string) =>
    call<string>("storyclub_open_year_dir", { year }),
  /** 期数封面（未生成过则报错，前端显示占位；浏览器模式返回二进制流 URL） */
  storyclubGetCoverDataUrl: (issueId: string) =>
    isWeb
      ? Promise.resolve(imgUrl(["story-cover", issueId]))
      : call<string>("storyclub_get_cover_data_url", { issue_id: issueId }),
  /** 批量取封面（浏览器模式直接映射为流 URL，免批量 IPC；桌面返回 [id, dataUrl][]） */
  storyclubGetCoversBatch: (issueIds: string[]) =>
    isWeb
      ? Promise.resolve(
          issueIds.map((id) => [id, imgUrl(["story-cover", id])] as [string, string]),
        )
      : call<[string, string][]>("storyclub_get_covers_batch", { issue_ids: issueIds }),
  /** 上传期数封面（打开阅读器时用 pdf.js 渲染第一页生成） */
  storyclubUploadCover: (issueId: string, dataUrl: string) =>
    call<null>("storyclub_upload_cover", { issue_id: issueId, data_url_value: dataUrl }),
  /** 后端直接提取第一页 JPEG（扫描件封面页多为整页图，秒级；失败时前端退回 pdf.js 渲染） */
  storyclubFirstPageJpeg: (issueId: string) =>
    isWeb
      ? Promise.resolve(imgUrl(["story-first", issueId]))
      : call<string>("storyclub_first_page_jpeg", { issue_id: issueId }),
  storyclubMissingCovers: () => call<string[]>("storyclub_missing_covers"),

  // ════════════ 局域网访问（仅桌面窗口展示用） ════════════
  getWebUrls: () => call<{ urls: string[]; password: string }>("get_web_urls"),
  openWebUrl: (url: string) => call<void>("open_web_url", { url }),
};
