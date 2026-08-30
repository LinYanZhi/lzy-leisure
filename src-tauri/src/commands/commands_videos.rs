//! 视频管理命令（扫描/路径/查询/封面）。原 commands.rs 拆分，外部路径 `commands::xxx` 不变。
use super::{covers_dir, data_url, decode_data_url, mime_from_ext};
use crate::db::videos as video_db;
use crate::video_scanner;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tauri::Manager;

// ── 视频：扫描导入 ──

#[derive(Serialize)]
pub struct ScanResult {
    pub total: i64,
    pub new_count: i64,
    pub duplicate_count: i64,
    pub new_videos: Vec<video_db::Video>,
}

/// 扫描目录并登记新视频（提取元数据，root_dir=dir）
#[tauri::command]
pub(crate) async fn scan_directory(dir: String) -> Result<ScanResult, String> {
    tauri::async_runtime::spawn_blocking(move || scan_video_dir_impl(Path::new(&dir)))
        .await
        .map_err(|e| format!("扫描任务失败: {e}"))?
}

/// 共享扫描实现：登记视频（root_dir=dir）、补登记字幕
fn scan_video_dir_impl(dir: &Path) -> Result<ScanResult, String> {
    let root = dir.to_string_lossy().to_string();
    let files = video_scanner::scan_videos(dir)?;

    let mut new_videos: Vec<video_db::Video> = Vec::new();
    let mut new_count = 0i64;
    let mut duplicate_count = 0i64;
    let now = crate::db::now();

    for (path, title) in files {
        if let Some(existing) = video_db::get_video_by_path(&path)? {
            duplicate_count += 1;
            // 已存在的视频：补登记/更新字幕与归属路径（如上次扫描后新增字幕文件）
            let sub = video_scanner::find_subtitle(Path::new(&path)).unwrap_or_default();
            video_db::update_video_scan_meta(&existing.id, &sub, &root)?;
            continue;
        }
        // 稳定 id：随机生成（不依赖路径，文件改名/移动后关联不丢）
        let id = crate::db::new_video_id();
        // 文件名解析：清洗标题杂质，提取年份/集数（标题为空则回退原始文件名）
        let parsed = video_scanner::parse_video_filename(&title);
        let clean_title = if parsed.title.is_empty() {
            title.clone()
        } else {
            parsed.title.clone()
        };
        let mut video = video_db::Video {
            id: id.clone(),
            title: clean_title,
            path: path.clone(),
            description: String::new(),
            license_plate: String::new(),
            cover: String::new(),
            duration: String::new(),
            file_size: String::new(),
            file_type: String::new(),
            fps: None,
            frame_width: None,
            frame_height: None,
            sort_order: 0,
            actors: Vec::new(),
            tags: Vec::new(),
            kinds: Vec::new(),
            subtitle_path: video_scanner::find_subtitle(Path::new(&path)).unwrap_or_default(),
            root_dir: root.clone(),
            series_id: String::new(),
            progress: 0.0,
            year: parsed.year.clone(),
            rating: 0.0,
            episode: parsed.episode.clone(),
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        // 元数据（时长/分辨率等）由前端按需补全（update_video_media_meta），扫描阶段不提取
        // 自动推断种类（竖屏/短视频/电影/动漫），可在播放器编辑面板手动修改
        video.kinds = video_scanner::infer_kinds(
            &path,
            &video.title,
            &video.duration,
            video.frame_width,
            video.frame_height,
        );
        video_db::upsert_video(&video)?;
        new_count += 1;
        new_videos.push(video);
    }

    Ok(ScanResult {
        total: new_count + duplicate_count,
        new_count,
        duplicate_count,
        new_videos,
    })
}

// ── 视频：导入路径（源）管理 ──

/// 路径相等（大小写不敏感，忽略分隔符写法差异）
fn video_paths_equal(a: &str, b: &str) -> bool {
    let parts = |p: &str| {
        Path::new(p)
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
            .collect::<Vec<String>>()
    };
    parts(a) == parts(b)
}

/// 去重：仅拒绝完全相同路径的重复导入（包含关系可安全共存，扫描器幂等不重复登记）
fn dedupe_video_root(new_path: &str) -> Result<(), String> {
    let roots = video_db::get_video_roots()?;
    if roots.iter().any(|r| video_paths_equal(&r.path, new_path)) {
        return Err(format!("「{new_path}」已在视频目录列表中，无需重复导入"));
    }
    Ok(())
}

/// 添加视频导入路径：登记路径 + 后台扫描该目录（含子目录）下全部视频。
/// 先登记路径立即返回，扫描在后台线程执行（元数据由前端按需补全，不依赖 ffmpeg），
/// 完成后通过 video-scan-done 事件通知前端刷新，
/// 避免界面长时间无响应；即使扫描失败路径也会留在源管理中，可单独重扫。
#[tauri::command]
pub(crate) async fn add_video_root(path: String) -> Result<String, String> {
    if !Path::new(&path).is_dir() {
        return Err(format!("目录不存在: {path}"));
    }
    dedupe_video_root(&path)?;
    let path2 = path.clone();
    video_db::add_video_root(&path2)?;
    tauri::async_runtime::spawn_blocking(move || {
        let result = scan_video_dir_impl(Path::new(&path2));
        match result {
            Ok(r) => {
                crate::events::emit(
                    "video-scan-done",
                    serde_json::json!({
                        "root_dir": path2,
                        "new_count": r.new_count,
                        "duplicate_count": r.duplicate_count,
                    }),
                );
            }
            Err(e) => {
                log::error!("扫描视频目录失败 {path2}: {e}");
            }
        }
    });
    Ok(format!("已登记视频目录 {path}，后台开始扫描"))
}

/// 移除视频导入路径：删除路径记录及其下全部视频（演员/标签关联级联删除）
#[tauri::command]
pub(crate) fn remove_video_root(path: String) -> Result<(), String> {
    video_db::remove_video_root(&path)
}

#[tauri::command]
pub(crate) fn get_video_roots() -> Result<Vec<video_db::VideoRootDir>, String> {
    video_db::get_video_roots()
}

/// 重新扫描单个导入路径：更新元数据/字幕，并移除磁盘上已不存在的视频
#[tauri::command]
pub(crate) async fn rescan_video_root(path: String) -> Result<String, String> {
    let path2 = path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if !Path::new(&path2).is_dir() {
            return Err(format!("目录不存在: {path2}"));
        }
        let files = video_scanner::scan_videos(Path::new(&path2))?;
        let keep: Vec<String> = files.iter().map(|(p, _)| p.clone()).collect();
        let result = scan_video_dir_impl(Path::new(&path2))?;
        let removed = video_db::delete_videos_not_in(&path2, &keep)?;
        crate::events::emit(
            "video-scan-done",
            serde_json::json!({
                "root_dir": path2,
                "new_count": result.new_count,
                "duplicate_count": result.duplicate_count,
                "removed": removed,
            }),
        );
        Ok::<String, String>(format!(
            "重新扫描完成：新增 {new} 个视频，重复 {dup} 个，移除 {removed} 个",
            new = result.new_count,
            dup = result.duplicate_count
        ))
    })
    .await
    .map_err(|e| format!("扫描任务失败: {e}"))?
}

/// 重新扫描全部视频导入路径
#[tauri::command]
pub(crate) async fn rescan_all_video_roots() -> Result<String, String> {
    let roots = video_db::get_video_roots()?;
    if roots.is_empty() {
        return Err("没有已添加的视频目录".to_string());
    }
    let paths: Vec<String> = roots.into_iter().map(|r| r.path).collect();
    tauri::async_runtime::spawn_blocking(move || {
        let mut total_new = 0i64;
        let mut total_dup = 0i64;
        let mut errors: Vec<String> = Vec::new();
        for path in &paths {
            match video_scanner::scan_videos(Path::new(path)) {
                Ok(files) => {
                    let keep: Vec<String> = files.iter().map(|(p, _)| p.clone()).collect();
                    match scan_video_dir_impl(Path::new(path)) {
                        Ok(r) => {
                            total_new += r.new_count;
                            total_dup += r.duplicate_count;
                            if let Err(e) = video_db::delete_videos_not_in(path, &keep) {
                                errors.push(format!("{path}: {e}"));
                            }
                        }
                        Err(e) => errors.push(format!("{path}: {e}")),
                    }
                }
                Err(e) => errors.push(format!("{path}: {e}")),
            }
        }
        crate::events::emit(
            "video-scan-done",
            serde_json::json!({
                "root_dir": "",
                "new_count": total_new,
                "duplicate_count": total_dup,
            }),
        );
        if errors.is_empty() {
            Ok::<String, String>(format!(
                "全部目录扫描完成：新增 {total_new} 个视频，重复 {total_dup} 个"
            ))
        } else {
            Err(errors.join("\n"))
        }
    })
    .await
    .map_err(|e| format!("扫描任务失败: {e}"))?
}

/// 手动添加单个视频（指定路径，登记并提取元数据）
#[tauri::command]
pub(crate) async fn add_video(path: String) -> Result<video_db::Video, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if !Path::new(&path).is_file() {
            return Err(format!("文件不存在: {path}"));
        }
        if video_db::get_video_by_path(&path)?.is_some() {
            return Err("该视频已在书库中".to_string());
        }
        // 稳定 id：随机生成（不依赖路径，文件改名/移动后关联不丢）
        let id = crate::db::new_video_id();
        let title = Path::new(&path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "未命名".to_string());
        // 文件名解析：清洗标题杂质，提取年份/集数（标题为空则回退原始文件名）
        let parsed = video_scanner::parse_video_filename(&title);
        let clean_title = if parsed.title.is_empty() {
            title
        } else {
            parsed.title.clone()
        };
        let now = crate::db::now();
        let mut video = video_db::Video {
            id: id.clone(),
            title: clean_title,
            path: path.clone(),
            description: String::new(),
            license_plate: String::new(),
            cover: String::new(),
            duration: String::new(),
            file_size: String::new(),
            file_type: String::new(),
            fps: None,
            frame_width: None,
            frame_height: None,
            sort_order: 0,
            actors: Vec::new(),
            tags: Vec::new(),
            kinds: Vec::new(),
            subtitle_path: video_scanner::find_subtitle(Path::new(&path)).unwrap_or_default(),
            root_dir: String::new(),
            series_id: String::new(),
            progress: 0.0,
            year: parsed.year.clone(),
            rating: 0.0,
            episode: parsed.episode.clone(),
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        // 元数据由前端按需补全（update_video_media_meta），添加时仅登记文件
        // 自动推断种类（竖屏/短视频/电影/动漫），可在播放器编辑面板手动修改
        video.kinds = video_scanner::infer_kinds(
            &path,
            &video.title,
            &video.duration,
            video.frame_width,
            video.frame_height,
        );
        video_db::upsert_video(&video)?;
        Ok(video)
    })
    .await
    .map_err(|e| format!("添加视频失败: {e}"))?
}

// ── 视频：查询与编辑 ──

#[derive(Default, Deserialize)]
pub struct VideoQuery {
    pub title: Option<String>,
    #[serde(default)]
    pub tag_ids: Vec<String>,
    pub match_all: Option<bool>,
    #[serde(default)]
    pub actor_ids: Vec<String>,
    pub vertical_only: Option<bool>,
    #[serde(default)]
    pub kinds: Vec<String>,
    /// 按导入路径（源）过滤；空/缺省表示全部
    #[serde(default)]
    pub root_dir: Option<String>,
    /// 按剧集过滤；空/缺省表示全部，"unassigned" 表示未归入任何剧集
    #[serde(default)]
    pub series_id: Option<String>,
}

#[tauri::command]
pub(crate) fn list_videos(query: Option<VideoQuery>) -> Result<Vec<video_db::Video>, String> {
    let q = query.unwrap_or_default();
    let filter = video_db::VideoFilter {
        title: q.title.filter(|t| !t.is_empty()),
        tag_ids: q.tag_ids,
        match_all: q.match_all.unwrap_or(false),
        actor_ids: q.actor_ids,
        vertical_only: q.vertical_only.unwrap_or(false),
        kinds: q.kinds,
        root_dir: q.root_dir.filter(|p| !p.is_empty()),
        series_id: q.series_id.filter(|s| !s.is_empty()),
    };
    video_db::list_videos(&filter)
}

#[tauri::command]
pub(crate) fn get_video(video_id: String) -> Result<Option<video_db::Video>, String> {
    video_db::get_video(&video_id)
}

/// 读取视频的同名字幕文件内容（UTF-8 文本；ass/srt 常见 GBK 自动转码）
#[tauri::command]
pub(crate) fn get_video_subtitle(video_id: String) -> Result<String, String> {
    let video = video_db::get_video(&video_id)?.ok_or_else(|| "视频不存在".to_string())?;
    if video.subtitle_path.is_empty() {
        return Err("该视频没有同名字幕文件".to_string());
    }
    let bytes = std::fs::read(&video.subtitle_path)
        .map_err(|e| format!("读取字幕文件失败: {e}"))?;
    Ok(video_scanner::decode_subtitle(&bytes))
}

#[derive(Deserialize)]
pub struct VideoEdit {
    pub id: String,
    pub title: String,
    pub description: String,
    pub license_plate: String,
    pub year: String,
    pub rating: f64,
    pub kinds: Vec<String>,
    pub actor_ids: Vec<String>,
    pub tag_ids: Vec<String>,
}

#[tauri::command]
pub(crate) fn update_video(edit: VideoEdit) -> Result<(), String> {
    video_db::update_video_editable(
        &edit.id,
        &edit.title,
        &edit.description,
        &edit.license_plate,
        &edit.year,
        edit.rating,
        &edit.kinds,
        &edit.actor_ids,
        &edit.tag_ids,
    )
}

/// 前端按需补全视频媒体元数据（时长/分辨率）。
/// 文件大小与格式在后端 stat 文件得到，不依赖 ffmpeg；
/// 若视频当前无自动种类，则用新元数据重新推断（用户已手动调整时保留）。
#[tauri::command]
pub(crate) fn update_video_media_meta(
    video_id: String,
    duration: String,
    frame_width: Option<i64>,
    frame_height: Option<i64>,
) -> Result<(), String> {
    let video = video_db::get_video(&video_id)?.ok_or_else(|| "视频不存在".to_string())?;
    let meta = std::fs::metadata(&video.path).map_err(|e| format!("读取文件信息失败: {e}"))?;
    let size_bytes = meta.len();
    let file_size = if size_bytes >= 1024 * 1024 * 1024 {
        format!("{:.2}GB", size_bytes as f64 / 1024.0 / 1024.0 / 1024.0)
    } else {
        format!("{:.2}MB", size_bytes as f64 / 1024.0 / 1024.0)
    };
    let file_type = Path::new(&video.path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let kinds: Option<Vec<String>> = if video.kinds.is_empty() {
        Some(video_scanner::infer_kinds(
            &video.path,
            &video.title,
            &duration,
            frame_width,
            frame_height,
        ))
    } else {
        None
    };
    video_db::update_video_media_meta(
        &video_id,
        &duration,
        &file_size,
        &file_type,
        frame_width,
        frame_height,
        kinds.as_deref(),
    )
}

#[tauri::command]
pub(crate) fn delete_video(video_id: String) -> Result<(), String> {
    video_db::delete_video(&video_id)
}

#[tauri::command]
pub(crate) fn batch_set_video_sort_order(orders: Vec<(i64, String)>) -> Result<(), String> {
    video_db::batch_set_sort_order(&orders)
}

/// 播放：允许 asset scope 并返回本地路径（前端 convertFileSrc 生成流 URL）。
/// 仅桌面模式使用；浏览器模式由 http::video_stream 提供 HTTP 流。
#[tauri::command]
pub(crate) fn get_video_play_path(video_id: String) -> Result<String, String> {
    let video = video_db::get_video(&video_id)?.ok_or_else(|| "视频不存在".to_string())?;
    let p = Path::new(&video.path);
    if !p.is_file() {
        return Err(format!("视频文件不存在: {}", video.path));
    }
    crate::app_handle()
        .asset_protocol_scope()
        .allow_file(p)
        .map_err(|e| format!("允许访问失败: {e}"))?;
    Ok(video.path)
}

/// 打开视频所在文件夹
#[tauri::command]
pub(crate) fn open_video_folder(video_id: String) -> Result<(), String> {
    let video = video_db::get_video(&video_id)?.ok_or_else(|| "视频不存在".to_string())?;
    let p = Path::new(&video.path);
    let dir = if p.is_dir() {
        p.to_path_buf()
    } else {
        p.parent().map(|d| d.to_path_buf()).ok_or_else(|| "无法定位目录".to_string())?
    };
    opener::open(dir).map_err(|e| format!("打开目录失败: {e}"))
}

// ── 视频：封面 ──

/// 读取视频同目录的手动封面（poster.jpg/png 或「视频同名.jpg/png」）。
/// 仅只读兼容：用户手动放置或旧版本落盘遗留的封面；不写入、不创建任何文件。
fn read_poster(video_path: &str) -> Option<(Vec<u8>, String)> {
    let p = Path::new(video_path);
    let dir = p.parent()?;
    let stem = p.file_stem()?.to_string_lossy().to_string();
    for name in [
        format!("{stem}.jpg"),
        format!("{stem}.png"),
        "poster.jpg".to_string(),
        "poster.png".to_string(),
    ] {
        let cand = dir.join(&name);
        if cand.is_file() {
            if let Ok(bytes) = std::fs::read(&cand) {
                let mime = if name.ends_with(".png") { "image/png" } else { "image/jpeg" };
                return Some((bytes, mime.to_string()));
            }
        }
    }
    None
}

/// 视频封面字节（HTTP 流式接口与 data URL 命令共用，只读已有封面）
/// 未命中任何已生成封面时返回 Err("封面未生成")，由前端 JS 截帧生成后调用 upload_cover 保存
pub(crate) fn video_cover_bytes(video_id: &str) -> Result<(String, Vec<u8>), String> {
    let video = video_db::get_video(video_id)?.ok_or_else(|| "视频不存在".to_string())?;

    // 0. 视频目录封面库优先（.leisure-video-covers.db，随文件走）
    if let Some((bytes, mime)) = crate::video_covers::load_cover(&video.path) {
        return Ok((mime, bytes));
    }

    // 1. 视频同目录手动封面（poster.jpg / {视频名}.jpg，只读兼容）
    if let Some((bytes, mime)) = read_poster(&video.path) {
        return Ok((mime, bytes));
    }

    // 2. 已有封面文件（应用缓存，旧数据/兜底）
    if !video.cover.is_empty() {
        let p = crate::app_data_dir().join(&video.cover);
        if p.is_file() {
            if let Ok(bytes) = std::fs::read(&p) {
                return Ok((mime_from_ext(&video.cover), bytes));
            }
        }
    }

    // 3. 默认封面路径（兜底）
    let default_cover = covers_dir()?.join(format!("{video_id}.jpg"));
    if default_cover.is_file() {
        if let Ok(bytes) = std::fs::read(&default_cover) {
            return Ok(("image/jpeg".to_string(), bytes));
        }
    }

    // 4. 未生成：无 ffmpeg 依赖，前端 JS 截帧后经 upload_cover 保存
    if !Path::new(&video.path).is_file() {
        return Err("视频文件不存在".to_string());
    }
    Err("封面未生成".to_string())
}

#[tauri::command]
pub(crate) async fn get_video_cover_data_url(video_id: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (mime, bytes) = video_cover_bytes(&video_id)?;
        Ok(data_url(&mime, &bytes))
    })
    .await
    .map_err(|e| format!("封面生成失败: {e}"))?
}

/// 是否已有可用封面（浏览器模式前端据此决定是否 JS 截帧；桌面模式可用失败信号替代）
#[tauri::command]
pub(crate) fn has_video_cover(video_id: String) -> bool {
    video_cover_bytes(&video_id).is_ok()
}

/// 上传视频封面（data URL）
#[tauri::command]
pub(crate) fn upload_cover(video_id: String, data_url_value: String) -> Result<(), String> {
    let video = video_db::get_video(&video_id)?.ok_or_else(|| "视频不存在".to_string())?;
    let (mime, bytes) = decode_data_url(&data_url_value).ok_or_else(|| "图片数据无效".to_string())?;
    let ext = if mime.contains("png") { "png" } else { "jpg" };
    let cover = covers_dir()?.join(format!("{video_id}.{ext}"));
    std::fs::write(&cover, &bytes).map_err(|e| format!("保存封面失败: {e}"))?;
    let rel = format!("covers/{video_id}.{ext}");
    video_db::set_video_cover(&video_id, &rel)?;
    // 显式上传 → 同步写入视频目录封面库（随文件走；失败静默降级）
    let _ = crate::video_covers::save_cover(&video.path, &bytes, &mime, 0, 0);
    Ok(())
}
