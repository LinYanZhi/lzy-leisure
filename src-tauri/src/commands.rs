//! 休闲时光业务命令（前端 invoke 调用）
//!
//! 按业务域拆分（外部路径 `commands::xxx` 不变，见文末 pub(crate) use 重导出）：
//!   - `commands_comics.rs`   漫画书架（根目录/查询/封面/进度）
//!   - `commands_videos.rs`   视频管理（扫描/路径/查询/封面）
//!   - `commands_series.rs`   演员/标签/剧集（Series）
//!   - `commands_novels.rs`   小说（EPUB）
//!   - `commands_storyclub.rs` 故事会
//! 本文件保留跨域共享的 data_url 工具。

pub(crate) mod commands_comics;
pub(crate) mod commands_videos;
pub(crate) mod commands_series;
pub(crate) mod commands_novels;
pub(crate) mod commands_storyclub;

use base64::Engine as _;
use std::path::{Path, PathBuf};

pub(crate) fn data_url(mime: &str, bytes: &[u8]) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("data:{mime};base64,{b64}")
}

pub(crate) fn decode_data_url(data_url: &str) -> Option<(String, Vec<u8>)> {
    let (head, b64) = data_url.split_once(',')?;
    let mime = head
        .trim_start_matches("data:")
        .split(';')
        .next()
        .unwrap_or("image/jpeg")
        .to_string();
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    Some((mime, bytes))
}

pub(crate) fn mime_from_ext(name: &str) -> String {
    let ext = Path::new(name)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png".to_string(),
        "webp" => "image/webp".to_string(),
        "gif" => "image/gif".to_string(),
        "jpg" | "jpeg" => "image/jpeg".to_string(),
        _ => "image/jpeg".to_string(),
    }
}

/// app_data_dir 下的封面目录
pub(crate) fn covers_dir() -> Result<PathBuf, String> {
    let dir = crate::app_data_dir().join("covers");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建封面目录失败: {e}"))?;
    Ok(dir)
}

// ══════════════════════════════════════════════════════════
//  子模块重导出（保持外部路径 commands::xxx 不变）
// ══════════════════════════════════════════════════════════
pub(crate) use commands_comics::{
    add_root_dir, batch_set_sort_order, clear_cover_cache, comic_cover_bytes, comic_page_bytes,
    comic_preview_bytes, delete_comic, get_all_completed_status, get_all_series_progress,
    get_chapters, get_comic, get_comic_config, get_comic_cover_data_url, get_comic_page_index,
    get_comic_pdf_data, get_completed_status, get_page_data_url, get_page_preview_data_url,
    get_reading_progress, get_root_dirs, get_series_progress, get_top_level_comics, list_pages,
    open_comic_directory, remove_root_dir, rename_comic, rescan_all, rescan_comic,
    reset_comic_cover, scan_root_dir, set_comic_config, set_comic_cover_from_offset,
    set_completed_status, set_reading_progress, sync_index, upload_comic_cover,
};
pub(crate) use commands_videos::{
    add_video, add_video_root, batch_set_video_sort_order, delete_video, get_video,
    get_video_cover_data_url, get_video_play_path, get_video_roots, get_video_subtitle,
    has_video_cover, list_videos, open_video_folder, remove_video_root, rescan_all_video_roots,
    rescan_video_root, scan_directory, update_video, update_video_media_meta, upload_cover,
    video_cover_bytes, VideoEdit, VideoQuery,
};
pub(crate) use commands_series::{
    actor_image_bytes, create_series, create_series_from_dir, delete_actor, delete_series,
    delete_tag, delete_tag_group, get_actor_image_data_url, get_series, list_actors, list_series,
    list_tag_groups, list_tags, save_actor, save_tag, save_tag_group, set_videos_series,
    update_series, update_video_progress, upload_actor_image, ActorInput, TagInput,
};
pub(crate) use commands_novels::{
    add_novel, add_novel_root, delete_novel, get_novel, get_novel_chapter_content,
    get_novel_cover_data_url, get_novel_roots, list_novels, novel_cover_bytes, open_novel_folder,
    remove_novel_root, rename_novel, rescan_novel_root, set_novel_progress,
};
pub(crate) use commands_storyclub::{
    storyclub_cover_bytes, storyclub_first_page_bytes, storyclub_first_page_jpeg,
    storyclub_get_cover_data_url, storyclub_get_covers_batch, storyclub_get_pdf_data,
    storyclub_get_pdf_path, storyclub_get_root, storyclub_list_issues, storyclub_missing_covers,
    storyclub_open_folder, storyclub_open_year_dir, storyclub_page_bytes, storyclub_remove_root,
    storyclub_rescan, storyclub_set_progress, storyclub_set_root, storyclub_update_page_count,
    storyclub_upload_cover,
};
