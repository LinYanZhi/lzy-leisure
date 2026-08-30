//! HTTP 服务层——命令分发。
//! 原 http.rs 拆分（路由见 http.rs，鉴权见 http_auth，流/浏览见 http_stream）：
//! POST /api/{command} 与 tauri command 同名，统一分发到 commands 模块。
use axum::{
    extract::Path as AxumPath,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use serde_json::{json, Value};

// ══════════════════════════════════════════════════════════
//  命令分发
// ══════════════════════════════════════════════════════════

pub(crate) fn err_json(e: String) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "ok": false, "error": e })),
    )
        .into_response()
}

fn req_str(v: &Value, k: &str) -> Result<String, String> {
    v.get(k)
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("缺少参数: {k}"))
}

fn req_i64(v: &Value, k: &str) -> Result<i64, String> {
    v.get(k)
        .and_then(|x| x.as_i64())
        .ok_or_else(|| format!("缺少参数: {k}"))
}

fn req_bool(v: &Value, k: &str) -> Result<bool, String> {
    v.get(k)
        .and_then(|x| x.as_bool())
        .ok_or_else(|| format!("缺少参数: {k}"))
}

fn req_opt<T: serde::de::DeserializeOwned>(v: &Value, k: &str) -> Result<Option<T>, String> {
    match v.get(k) {
        Some(Value::Null) | None => Ok(None),
        Some(x) => serde_json::from_value(x.clone())
            .map(Some)
            .map_err(|e| format!("参数 {k} 格式错误: {e}")),
    }
}

fn req_orders(v: &Value) -> Result<Vec<(i64, String)>, String> {
    let arr = v
        .get("orders")
        .and_then(|x| x.as_array())
        .ok_or_else(|| "缺少参数: orders".to_string())?;
    let mut out = Vec::new();
    for item in arr {
        if let Some(pair) = item.as_array() {
            if pair.len() >= 2 {
                let a = pair[0].as_i64().unwrap_or(0);
                let b = pair[1].as_str().unwrap_or("").to_string();
                out.push((a, b));
            }
        }
    }
    Ok(out)
}

/// POST /api/{command}：与 tauri command 同名的统一分发入口
pub(crate) async fn api_command(
    AxumPath(cmd): AxumPath<String>,
    Json(payload): Json<Value>,
) -> Response {
    dispatch(&cmd, &payload).await
}

async fn dispatch(cmd: &str, p: &Value) -> Response {
    match inner_dispatch(cmd, p).await {
        Ok(data) => Json(json!({ "ok": true, "data": data })).into_response(),
        Err(e) => err_json(e),
    }
}

/// 命令分发（内部返回 Result，`?` 统一向上抛出错误）
async fn inner_dispatch(cmd: &str, p: &Value) -> Result<Value, String> {
    use crate::commands as c;
    match cmd {
        // ── 通用（lib.rs 顶层命令） ──
        "open_directory" => req_str(p, "path")
            .map(crate::open_directory)
            .and_then(flatten),
        "check_path_exists" => {
            req_str(p, "path").map(|path| Value::Bool(crate::check_path_exists(path)))
        }
        "get_app_version" => Ok(Value::String(crate::get_app_version())),
        "get_data_directory" => Ok(Value::String(crate::get_data_directory())),
        "get_install_directory" => Ok(Value::String(crate::get_install_directory())),
        "set_window_pin" => Err("浏览器模式不支持置顶窗口".to_string()),

        // ── 漫画：根目录 ──
        "add_root_dir" => req_str(p, "path").map(c::add_root_dir).and_then(flatten),
        "remove_root_dir" => req_str(p, "path").map(c::remove_root_dir).and_then(flatten),
        "get_root_dirs" => c::get_root_dirs().map(to_value),
        "scan_root_dir" => {
            let path = req_str(p, "path")?;
            c::scan_root_dir(path).await.map(Value::String)
        }
        "rescan_comic" => {
            let id = req_str(p, "comic_id")?;
            c::rescan_comic(id).await.map(Value::String)
        }
        "rescan_all" => c::rescan_all().await.map(Value::String),
        "clear_cover_cache" => c::clear_cover_cache().map(|_| Value::Null),

        // ── 漫画：查询与编辑 ──
        "get_top_level_comics" => c::get_top_level_comics().map(to_value),
        "get_chapters" => req_str(p, "series_id")
            .map(c::get_chapters)
            .and_then(flatten),
        "get_comic" => req_str(p, "comic_id").map(c::get_comic).and_then(flatten),
        "delete_comic" => req_str(p, "comic_id").map(c::delete_comic).and_then(flatten),
        "rename_comic" => {
            let id = req_str(p, "comic_id")?;
            let title = req_str(p, "title")?;
            c::rename_comic(id, title).map(|_| Value::Null)
        }
        "open_comic_directory" => {
            req_str(p, "comic_id").map(c::open_comic_directory).and_then(flatten)
        }
        "batch_set_sort_order" => req_orders(p).map(c::batch_set_sort_order).and_then(flatten),

        // ── 漫画：进度 & 完结 ──
        "get_series_progress" => {
            req_str(p, "series_id").map(c::get_series_progress).and_then(flatten)
        }
        "get_all_series_progress" => c::get_all_series_progress().map(to_value),
        "get_completed_status" => {
            req_str(p, "comic_id").map(c::get_completed_status).and_then(flatten)
        }
        "set_completed_status" => {
            let id = req_str(p, "comic_id")?;
            let completed = req_bool(p, "completed")?;
            c::set_completed_status(id, completed).map(|_| Value::Null)
        }
        "get_all_completed_status" => c::get_all_completed_status().map(to_value),
        "sync_index" => req_str(p, "comic_id").map(c::sync_index).and_then(flatten),

        // ── 漫画：封面 & 页面 ──
        "get_comic_cover_data_url" => {
            let id = req_str(p, "comic_id")?;
            c::get_comic_cover_data_url(id).await.map(Value::String)
        }
        "list_pages" => req_str(p, "comic_id").map(c::list_pages).and_then(flatten),
        "get_page_data_url" => {
            let id = req_str(p, "comic_id")?;
            let idx = req_i64(p, "page_index")? as usize;
            c::get_page_data_url(id, idx).await.map(Value::String)
        }
        "get_comic_page_index" => {
            let id = req_str(p, "comic_id")?;
            c::get_comic_page_index(id).await.map(to_value)
        }
        "get_page_preview_data_url" => {
            let id = req_str(p, "comic_id")?;
            let idx = req_i64(p, "page_index")? as usize;
            c::get_page_preview_data_url(id, idx).await.map(Value::String)
        }
        "set_comic_cover_from_offset" => {
            let id = req_str(p, "comic_id")?;
            let offset = req_i64(p, "offset")?;
            c::set_comic_cover_from_offset(id, offset)
                .await
                .map(|_| Value::Null)
        }
        "reset_comic_cover" => req_str(p, "comic_id")
            .map(c::reset_comic_cover)
            .and_then(flatten),
        "get_comic_pdf_data" => {
            let id = req_str(p, "comic_id")?;
            c::get_comic_pdf_data(id).await.map(Value::String)
        }
        "upload_comic_cover" => {
            let id = req_str(p, "comic_id")?;
            let data = req_str(p, "data_url_value")?;
            let overwrite = p
                .get("overwrite")
                .and_then(|x| x.as_bool())
                .unwrap_or(false);
            c::upload_comic_cover(id, data, Some(overwrite)).map(Value::String)
        }

        // ── 漫画：进度 & 配置 ──
        "set_reading_progress" => {
            let id = req_str(p, "comic_id")?;
            let page = req_i64(p, "page")?;
            c::set_reading_progress(id, page).map(|_| Value::Null)
        }
        "get_reading_progress" => req_str(p, "comic_id")
            .map(c::get_reading_progress)
            .and_then(flatten),
        "get_comic_config" => {
            req_str(p, "comic_id").map(c::get_comic_config).and_then(flatten)
        }
        "set_comic_config" => {
            let id = req_str(p, "comic_id")?;
            let config: std::collections::HashMap<String, String> =
                req_opt(p, "config")?.unwrap_or_default();
            c::set_comic_config(id, config).map(|_| Value::Null)
        }

        // ── 视频：扫描 / 添加 ──
        "scan_directory" => {
            let dir = req_str(p, "dir")?;
            c::scan_directory(dir).await.map(to_value)
        }
        "add_video" => {
            let path = req_str(p, "path")?;
            c::add_video(path).await.map(to_value)
        }

        // ── 视频：查询与编辑 ──
        "list_videos" => {
            let q: Option<c::VideoQuery> = req_opt(p, "query")?;
            c::list_videos(q).map(to_value)
        }
        "get_video" => req_str(p, "video_id").map(c::get_video).and_then(flatten),
        "get_video_subtitle" => {
            req_str(p, "video_id").map(c::get_video_subtitle).and_then(flatten)
        }
        "get_video_roots" => Ok(c::get_video_roots().map(to_value)?),
        "add_video_root" => {
            let path = req_str(p, "path")?;
            c::add_video_root(path).await.map(Value::String)
        }
        "remove_video_root" => req_str(p, "path").map(c::remove_video_root).and_then(flatten),
        "rescan_video_root" => {
            let path = req_str(p, "path")?;
            c::rescan_video_root(path).await.map(Value::String)
        }
        "rescan_all_video_roots" => c::rescan_all_video_roots().await.map(Value::String),
        "update_video" => {
            let edit: c::VideoEdit = req_opt(p, "edit")?.ok_or_else(|| "缺少参数: edit".to_string())?;
            c::update_video(edit).map(|_| Value::Null)
        }
        "update_video_media_meta" => {
            let id = req_str(p, "video_id")?;
            let duration = req_str(p, "duration")?;
            let width = p.get("frame_width").and_then(|x| x.as_i64());
            let height = p.get("frame_height").and_then(|x| x.as_i64());
            c::update_video_media_meta(id, duration, width, height).map(|_| Value::Null)
        }
        "delete_video" => req_str(p, "video_id").map(c::delete_video).and_then(flatten),
        "batch_set_video_sort_order" => {
            req_orders(p).map(c::batch_set_video_sort_order).and_then(flatten)
        }
        "get_video_play_path" => {
            req_str(p, "video_id").map(c::get_video_play_path).and_then(flatten)
        }
        "open_video_folder" => Err("浏览器模式不支持打开文件夹".to_string()),

        // ── 视频：剧集（Series） ──
        "list_series" => Ok(c::list_series().map(to_value)?),
        "get_series" => req_str(p, "series_id").map(c::get_series).and_then(flatten),
        "create_series" => {
            let title = req_str(p, "title")?;
            let description = req_str(p, "description").unwrap_or_default();
            c::create_series(title, description).map(to_value)
        }
        "create_series_from_dir" => {
            let path = req_str(p, "path")?;
            c::create_series_from_dir(path).await.map(to_value)
        }
        "update_series" => {
            let id = req_str(p, "series_id")?;
            let title = req_str(p, "title")?;
            let description = req_str(p, "description").unwrap_or_default();
            c::update_series(id, title, description).map(|_| Value::Null)
        }
        "delete_series" => req_str(p, "series_id").map(c::delete_series).and_then(flatten),
        "set_videos_series" => {
            let sid = req_str(p, "series_id")?;
            let ids: Vec<String> = p
                .get("video_ids")
                .and_then(|x| x.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            c::set_videos_series(sid, ids).map(|_| Value::Null)
        }
        "update_video_progress" => {
            let id = req_str(p, "video_id")?;
            let secs = p.get("seconds").and_then(|x| x.as_f64()).ok_or_else(|| "缺少参数: seconds".to_string())?;
            c::update_video_progress(id, secs).map(|_| Value::Null)
        }

        // ── 视频：封面 ──
        "get_video_cover_data_url" => {
            let id = req_str(p, "video_id")?;
            c::get_video_cover_data_url(id).await.map(Value::String)
        }
        "has_video_cover" => {
            let id = req_str(p, "video_id")?;
            Ok(Value::Bool(c::has_video_cover(id)))
        }
        "upload_cover" => {
            let id = req_str(p, "video_id")?;
            let data = req_str(p, "data_url_value")?;
            c::upload_cover(id, data).map(|_| Value::Null)
        }

        // ── 视频：演员 ──
        "list_actors" => c::list_actors().map(to_value),
        "save_actor" => {
            let input: c::ActorInput = req_opt(p, "input")?.ok_or_else(|| "缺少参数: input".to_string())?;
            c::save_actor(input).map(to_value)
        }
        "delete_actor" => req_str(p, "actor_id").map(c::delete_actor).and_then(flatten),
        "upload_actor_image" => {
            let id = req_str(p, "actor_id")?;
            let data = req_str(p, "data_url_value")?;
            c::upload_actor_image(id, data).map(|_| Value::Null)
        }
        "get_actor_image_data_url" => {
            req_str(p, "actor_id")
                .map(c::get_actor_image_data_url)
                .and_then(flatten)
        }

        // ── 视频：标签组 / 标签 ──
        "list_tag_groups" => c::list_tag_groups().map(to_value),
        "save_tag_group" => {
            let name = req_str(p, "name")?;
            let id: Option<String> = req_opt(p, "id")?;
            c::save_tag_group(name, id).map(to_value)
        }
        "delete_tag_group" => req_str(p, "group_id").map(c::delete_tag_group).and_then(flatten),
        "list_tags" => c::list_tags().map(to_value),
        "save_tag" => {
            let input: c::TagInput = req_opt(p, "input")?.ok_or_else(|| "缺少参数: input".to_string())?;
            c::save_tag(input).map(to_value)
        }
        "delete_tag" => req_str(p, "tag_id").map(c::delete_tag).and_then(flatten),

        // ── 小说 ──
        "add_novel_root" => {
            let path = req_str(p, "path")?;
            c::add_novel_root(path).await.map(Value::String)
        }
        "rescan_novel_root" => {
            let path = req_str(p, "path")?;
            c::rescan_novel_root(path).await.map(Value::String)
        }
        "remove_novel_root" => req_str(p, "path").map(c::remove_novel_root).and_then(flatten),
        "get_novel_roots" => c::get_novel_roots().map(to_value),
        "add_novel" => req_str(p, "path").map(c::add_novel).and_then(flatten),
        "list_novels" => c::list_novels().map(to_value),
        "get_novel" => req_str(p, "novel_id").map(c::get_novel).and_then(flatten),
        "delete_novel" => req_str(p, "novel_id").map(c::delete_novel).and_then(flatten),
        "rename_novel" => {
            let id = req_str(p, "novel_id")?;
            let title = req_str(p, "title")?;
            c::rename_novel(id, title).map(|_| Value::Null)
        }
        "get_novel_cover_data_url" => {
            req_str(p, "novel_id").map(c::get_novel_cover_data_url).and_then(flatten)
        }
        "get_novel_chapter_content" => {
            let id = req_str(p, "novel_id")?;
            let idx = req_i64(p, "chapter_index")?;
            c::get_novel_chapter_content(id, idx).map(Value::String)
        }
        "set_novel_progress" => {
            let id = req_str(p, "novel_id")?;
            let chapter = req_i64(p, "chapter")?;
            let pos = req_i64(p, "pos")?;
            c::set_novel_progress(id, chapter, pos).map(|_| Value::Null)
        }
        "open_novel_folder" => req_str(p, "novel_id").map(c::open_novel_folder).and_then(flatten),

        // ── 故事会 ──
        "storyclub_set_root" => {
            let path = req_str(p, "path")?;
            c::storyclub_set_root(path).await.map(Value::String)
        }
        "storyclub_get_root" => c::storyclub_get_root().map(to_value),
        "storyclub_remove_root" => c::storyclub_remove_root().map(|_| Value::Null),
        "storyclub_rescan" => c::storyclub_rescan().await.map(Value::String),
        "storyclub_list_issues" => c::storyclub_list_issues().map(to_value),
        "storyclub_get_pdf_data" => {
            let id = req_str(p, "issue_id")?;
            c::storyclub_get_pdf_data(id).await.map(Value::String)
        }
        "storyclub_set_progress" => {
            let id = req_str(p, "issue_id")?;
            let page = req_i64(p, "page")?;
            c::storyclub_set_progress(id, page).map(|_| Value::Null)
        }
        "storyclub_update_page_count" => {
            let id = req_str(p, "issue_id")?;
            let count = req_i64(p, "page_count")?;
            c::storyclub_update_page_count(id, count).map(|_| Value::Null)
        }
        "storyclub_open_folder" => {
            req_str(p, "issue_id").map(c::storyclub_open_folder).and_then(flatten)
        }
        "storyclub_open_year_dir" => {
            req_str(p, "year").map(c::storyclub_open_year_dir).and_then(flatten)
        }
        "storyclub_get_cover_data_url" => {
            req_str(p, "issue_id")
                .map(c::storyclub_get_cover_data_url)
                .and_then(flatten)
        }
        "storyclub_get_covers_batch" => {
            let ids = req_opt::<Vec<String>>(p, "issue_ids")?.unwrap_or_default();
            c::storyclub_get_covers_batch(ids).await.map(to_value)
        }
        "storyclub_upload_cover" => {
            let id = req_str(p, "issue_id")?;
            let data = req_str(p, "data_url_value")?;
            c::storyclub_upload_cover(id, data).map(|_| Value::Null)
        }
        "storyclub_first_page_jpeg" => {
            let id = req_str(p, "issue_id")?;
            c::storyclub_first_page_jpeg(id).await.map(to_value)
        }
        "storyclub_missing_covers" => c::storyclub_missing_covers().map(to_value),

        _ => Err(format!("未知命令: {cmd}")),
    }
}

/// 将 Result<T, String>（T: Serialize）统一转为 Value
fn flatten<T: Serialize>(r: Result<T, String>) -> Result<Value, String> {
    r.map(|v| serde_json::to_value(v).unwrap_or(Value::Null))
}

/// 将任意 Serialize 值转为 JSON Value（序列化失败用 Null 兜底）
fn to_value<T: Serialize>(v: T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}
