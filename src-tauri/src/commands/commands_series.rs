//! 演员/标签/剧集（Series）命令。原 commands.rs 拆分，外部路径 `commands::xxx` 不变。
use super::{data_url, decode_data_url, mime_from_ext};
use crate::db::videos as video_db;
use crate::video_scanner;
use serde::{Deserialize, Serialize};
use std::path::Path;

// ── 演员 ──

#[derive(Deserialize)]
pub struct ActorInput {
    pub id: Option<String>,
    pub name: String,
    pub stage_names: Vec<String>,
    pub height: String,
    pub cup_size: String,
    pub birthdate: String,
    pub bio: String,
    pub sort_order: Option<i64>,
}

#[tauri::command]
pub(crate) fn list_actors() -> Result<Vec<video_db::Actor>, String> {
    video_db::list_actors()
}

#[tauri::command]
pub(crate) fn save_actor(input: ActorInput) -> Result<video_db::Actor, String> {
    let id = input.id.unwrap_or_else(|| crate::db::video_id(&input.name));
    let existing = video_db::get_actor(&id)?;
    let now = crate::db::now();
    let actor = video_db::Actor {
        id: id.clone(),
        name: input.name,
        stage_names: input.stage_names,
        height: input.height,
        cup_size: input.cup_size,
        birthdate: input.birthdate,
        bio: input.bio,
        images: existing.as_ref().map(|a| a.images.clone()).unwrap_or_default(),
        sort_order: input.sort_order.unwrap_or(existing.as_ref().map(|a| a.sort_order).unwrap_or(0)),
        created_at: existing.as_ref().map(|a| a.created_at.clone()).unwrap_or_else(|| now.clone()),
        updated_at: now,
    };
    video_db::upsert_actor(&actor)?;
    Ok(actor)
}

#[tauri::command]
pub(crate) fn delete_actor(actor_id: String) -> Result<(), String> {
    video_db::delete_actor(&actor_id)
}

/// 上传演员头像（data URL）
#[tauri::command]
pub(crate) fn upload_actor_image(actor_id: String, data_url_value: String) -> Result<(), String> {
    let (mime, bytes) = decode_data_url(&data_url_value).ok_or_else(|| "图片数据无效".to_string())?;
    let dir = crate::app_data_dir().join("actor_images");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建目录失败: {e}"))?;
    let ext = if mime.contains("png") { "png" } else { "jpg" };
    let fname = format!("{actor_id}.{ext}");
    std::fs::write(dir.join(&fname), &bytes).map_err(|e| format!("保存头像失败: {e}"))?;

    let mut actor = video_db::get_actor(&actor_id)?.ok_or_else(|| "演员不存在".to_string())?;
    let rel = format!("actor_images/{fname}");
    if !actor.images.contains(&rel) {
        actor.images.push(rel);
    }
    actor.updated_at = crate::db::now();
    video_db::upsert_actor(&actor)
}

/// 演员头像字节（HTTP 流式接口与 data URL 命令共用）
pub(crate) fn actor_image_bytes(actor_id: &str) -> Result<(String, Vec<u8>), String> {
    let actor = video_db::get_actor(actor_id)?.ok_or_else(|| "演员不存在".to_string())?;
    let rel = actor.images.first().ok_or_else(|| "演员暂无头像".to_string())?;
    let p = crate::app_data_dir().join(rel);
    let bytes = std::fs::read(&p).map_err(|e| format!("读取头像失败: {e}"))?;
    Ok((mime_from_ext(rel), bytes))
}

/// 获取演员头像 data URL（读取 actor_images 目录下的第一张图）
#[tauri::command]
pub(crate) fn get_actor_image_data_url(actor_id: String) -> Result<String, String> {
    let (mime, bytes) = actor_image_bytes(&actor_id)?;
    Ok(data_url(&mime, &bytes))
}

// ── 标签组 / 标签 ──

#[tauri::command]
pub(crate) fn list_tag_groups() -> Result<Vec<video_db::TagGroup>, String> {
    video_db::list_tag_groups()
}

#[tauri::command]
pub(crate) fn save_tag_group(name: String, id: Option<String>) -> Result<video_db::TagGroup, String> {
    let gid = id.unwrap_or_else(|| crate::db::video_id(&name));
    let now = crate::db::now();
    let group = video_db::TagGroup {
        id: gid.clone(),
        name,
        sort_order: 0,
        created_at: now,
    };
    video_db::upsert_tag_group(&group)?;
    Ok(group)
}

#[tauri::command]
pub(crate) fn delete_tag_group(group_id: String) -> Result<(), String> {
    video_db::delete_tag_group(&group_id)
}

#[tauri::command]
pub(crate) fn list_tags() -> Result<Vec<video_db::Tag>, String> {
    video_db::list_tags()
}

#[derive(Deserialize)]
pub struct TagInput {
    pub id: Option<String>,
    pub name: String,
    pub color: String,
    pub group_id: String,
    pub sort_order: Option<i64>,
}

#[tauri::command]
pub(crate) fn save_tag(input: TagInput) -> Result<video_db::Tag, String> {
    let id = input.id.unwrap_or_else(|| crate::db::video_id(&input.name));
    let now = crate::db::now();
    let tag = video_db::Tag {
        id: id.clone(),
        name: input.name,
        color: if input.color.is_empty() { "#8a8a8a".into() } else { input.color },
        group_id: input.group_id,
        sort_order: input.sort_order.unwrap_or(0),
        created_at: now,
    };
    video_db::upsert_tag(&tag)?;
    Ok(tag)
}

#[tauri::command]
pub(crate) fn delete_tag(tag_id: String) -> Result<(), String> {
    video_db::delete_tag(&tag_id)
}

// ── 视频：剧集（Series） ──

/// 从已登记导入路径中推断 path 的归属（最长前缀匹配，大小写/分隔符不敏感）；无匹配返回空串
fn infer_video_root(path: &str) -> String {
    let roots = video_db::get_video_roots().unwrap_or_default();
    let pn = path.trim_end_matches(['\\', '/']);
    let mut best: Option<(usize, String)> = None;
    for r in roots {
        let rn = r.path.trim_end_matches(['\\', '/']);
        let pref = pn.as_bytes().get(..rn.len());
        let is_pref = pref.map(|s| s.eq_ignore_ascii_case(rn.as_bytes())).unwrap_or(false)
            && (pn.len() == rn.len()
                || pn.as_bytes().get(rn.len()).map(|c| *c == b'\\' || *c == b'/').unwrap_or(false));
        if is_pref && best.as_ref().map(|(l, _)| rn.len() > *l).unwrap_or(true) {
            best = Some((rn.len(), r.path.clone()));
        }
    }
    best.map(|(_, p)| p).unwrap_or_default()
}

/// 登记单个视频文件（不存在时创建记录并提取元数据/字幕/种类）；已存在时返回现有记录
fn register_video_file(path: &str, root_dir: &str) -> Result<video_db::Video, String> {
    if let Some(v) = video_db::get_video_by_path(path)? {
        return Ok(v);
    }
    // 稳定 id：随机生成（不依赖路径，文件改名/移动后关联不丢）
    let id = crate::db::new_video_id();
    let title = Path::new(path)
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
        path: path.to_string(),
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
        subtitle_path: video_scanner::find_subtitle(Path::new(path)).unwrap_or_default(),
        root_dir: root_dir.to_string(),
        series_id: String::new(),
        progress: 0.0,
        year: parsed.year.clone(),
        rating: 0.0,
        episode: parsed.episode.clone(),
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    // 元数据由前端按需补全（update_video_media_meta），添加时仅登记文件
    video.kinds = video_scanner::infer_kinds(
        path,
        &video.title,
        &video.duration,
        video.frame_width,
        video.frame_height,
    );
    video_db::upsert_video(&video)?;
    Ok(video)
}

#[tauri::command]
pub(crate) fn list_series() -> Result<Vec<video_db::Series>, String> {
    video_db::list_series()
}

/// 剧集详情：剧集信息 + 其下视频（按标题自然排序）
#[derive(Serialize)]
pub struct SeriesDetail {
    pub series: video_db::Series,
    pub videos: Vec<video_db::Video>,
}

#[tauri::command]
pub(crate) fn get_series(series_id: String) -> Result<SeriesDetail, String> {
    let series = video_db::get_series(&series_id)?.ok_or_else(|| "剧集不存在".to_string())?;
    let videos = video_db::get_series_videos(&series_id)?;
    Ok(SeriesDetail { series, videos })
}

#[tauri::command]
pub(crate) fn create_series(title: String, description: String) -> Result<video_db::Series, String> {
    video_db::create_series(&title, &description)
}

/// 目录一键成剧集：创建剧集（目录名作标题），该目录（含子目录）下全部视频登记并归入。
/// 对应「把一个目录作为一部动漫/剧集来管理」的典型场景。
#[tauri::command]
pub(crate) async fn create_series_from_dir(path: String) -> Result<video_db::Series, String> {
    if !Path::new(&path).is_dir() {
        return Err(format!("目录不存在: {path}"));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let title = Path::new(&path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        let series = video_db::create_series(&title, "")?;
        let files = video_scanner::scan_videos(Path::new(&path))?;
        if files.is_empty() {
            video_db::delete_series(&series.id)?;
            return Err(format!("「{path}」下没有找到视频文件"));
        }
        let root = infer_video_root(&path);
        let mut ids = Vec::with_capacity(files.len());
        for (fp, _) in &files {
            let v = register_video_file(fp, &root)?;
            ids.push(v.id);
        }
        video_db::set_videos_series(&series.id, &ids)?;
        Ok(series)
    })
    .await
    .map_err(|e| format!("创建剧集失败: {e}"))?
}

#[tauri::command]
pub(crate) fn update_series(
    series_id: String,
    title: String,
    description: String,
) -> Result<(), String> {
    video_db::update_series(&series_id, &title, &description)
}

#[tauri::command]
pub(crate) fn delete_series(series_id: String) -> Result<(), String> {
    video_db::delete_series(&series_id)
}

/// 批量设置视频归属：video_ids 归入指定剧集；series_id 传空串表示解除归属
#[tauri::command]
pub(crate) fn set_videos_series(series_id: String, video_ids: Vec<String>) -> Result<(), String> {
    video_db::set_videos_series(&series_id, &video_ids)
}

/// 保存播放进度（秒），用于续播
#[tauri::command]
pub(crate) fn update_video_progress(video_id: String, seconds: f64) -> Result<(), String> {
    video_db::update_video_progress(&video_id, seconds)
}
