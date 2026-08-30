//! HTTP 服务层——事件推送与文件/字节流。
//! 原 http.rs 拆分（路由见 http.rs，鉴权见 http_auth，命令分发见 http_dispatch）：
//! SSE 事件推送、目录浏览、视频/PDF Range 流、图片二进制流。
use super::http_dispatch::err_json;
use axum::{
    body::Body,
    extract::{Path as AxumPath, Query},
    http::{HeaderMap, Request, StatusCode},
    response::sse::{Event, KeepAlive, Sse},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use tower::ServiceExt;
use tower_http::services::ServeFile;

// ══════════════════════════════════════════════════════════
//  SSE 事件推送
// ══════════════════════════════════════════════════════════

pub(crate) async fn api_events() -> Response {
    let rx = crate::events::subscribe();
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(payload) => {
                    // payload: {"event":"scan-done","payload":{...}}
                    let frame = match serde_json::from_str::<Value>(&payload) {
                        Ok(v) => {
                            let name = v["event"].as_str().unwrap_or("message");
                            let data = v["payload"].to_string();
                            Event::default().event(name).data(data)
                        }
                        Err(_) => Event::default().event("message").data(payload),
                    };
                    return Some((Ok::<_, axum::Error>(frame), rx));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return None,
            }
        }
    });
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(20)))
        .into_response()
}

// ══════════════════════════════════════════════════════════
//  目录浏览（浏览器模式选目录）
// ══════════════════════════════════════════════════════════

#[derive(Deserialize)]
pub(crate) struct FsListQuery {
    path: Option<String>,
}

#[derive(Serialize)]
struct FsList {
    path: String,
    parent: Option<String>,
    dirs: Vec<FsEntry>,
    files: Vec<FsEntry>,
}

#[derive(Serialize)]
struct FsEntry {
    name: String,
    path: String,
}

pub(crate) async fn fs_list(Query(q): Query<FsListQuery>) -> Response {
    let path = q.path.unwrap_or_default().trim().to_string();

    // 空路径 → 列出盘符（Windows）
    if path.is_empty() {
        let mut dirs = Vec::new();
        // 并发的 is_dir 探测：不可达的映射网络盘（如离线 NAS）会阻塞等待网络超时，
        // 单盘 1s 无响应即跳过，避免目录选择器长时间无反应（"点击添加后很久才出现"）
        for letter in b'A'..=b'Z' {
            let drive = format!("{}:\\", letter as char);
            let probe_drive = drive.clone();
            let probe =
                tokio::task::spawn_blocking(move || Path::new(&probe_drive).is_dir());
            if matches!(
                tokio::time::timeout(std::time::Duration::from_millis(1000), probe).await,
                Ok(Ok(true))
            ) {
                dirs.push(FsEntry {
                    name: drive.clone(),
                    path: drive,
                });
            }
        }
        return Json(json!({
            "ok": true,
            "data": FsList {
                path: String::new(),
                parent: None,
                dirs,
                files: Vec::<FsEntry>::new(),
            }
        }))
        .into_response();
    }

    let p = Path::new(&path);
    if !p.is_dir() {
        return err_json(format!("目录不存在: {path}"));
    }
    let parent = p.parent().map(|d| d.to_string_lossy().to_string());
    // read_dir 同样可能阻塞（导航进不可达的网络目录），spawn_blocking + 超时兜底
    let scan_path = path.clone();
    let listed = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::task::spawn_blocking(move || {
            let mut dirs = Vec::new();
            let mut files = Vec::new();
            if let Ok(rd) = std::fs::read_dir(Path::new(&scan_path)) {
                let mut entries: Vec<_> = rd.flatten().collect();
                entries.sort_by_key(|e| e.file_name().to_string_lossy().to_lowercase());
                for e in entries {
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.starts_with('.') || name.starts_with('$') {
                        continue;
                    }
                    let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    if is_dir {
                        dirs.push(FsEntry {
                            name: name.clone(),
                            path: e.path().to_string_lossy().to_string(),
                        });
                    } else if is_video_ext(&name) {
                        files.push(FsEntry {
                            name: name.clone(),
                            path: e.path().to_string_lossy().to_string(),
                        });
                    }
                }
            }
            (dirs, files)
        }),
    )
    .await;

    let (dirs, files) = match listed {
        Ok(Ok(v)) => v,
        _ => return err_json(format!("读取目录超时或失败: {path}")),
    };
    Json(json!({
        "ok": true,
        "data": FsList {
            path: path.clone(),
            parent,
            dirs,
            files,
        }
    }))
    .into_response()
}

/// 常见视频扩展名（浏览器模式文件选择用）
fn is_video_ext(name: &str) -> bool {
    let ext = Path::new(name)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        "mp4" | "mkv" | "avi" | "mov" | "wmv" | "flv" | "webm" | "m4v" | "ts" | "rmvb"
            | "mpg" | "mpeg"
    )
}

// ══════════════════════════════════════════════════════════
//  视频流（HTTP Range，支持拖动进度条）
// ══════════════════════════════════════════════════════════

pub(crate) async fn video_stream(AxumPath(id): AxumPath<String>, headers: HeaderMap) -> Response {
    let video = match crate::db::videos::get_video(&id) {
        Ok(Some(v)) => v,
        _ => return (StatusCode::NOT_FOUND, "视频不存在").into_response(),
    };
    let p = Path::new(&video.path);
    if !p.is_file() {
        return (StatusCode::NOT_FOUND, "视频文件不存在").into_response();
    }
    // 把原始请求头（Range 等）转发给 ServeFile，支持拖动进度条
    let mut req = Request::new(Body::empty());
    *req.headers_mut() = headers;
    match ServeFile::new(p.to_path_buf()).oneshot(req).await {
        Ok(resp) => resp.into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("视频流失败: {e}"),
        )
            .into_response(),
    }
}

// ══════════════════════════════════════════════════════════
//  故事会 PDF 流（HTTP Range，pdf.js 分块按需拉取，免全量 base64）
// ══════════════════════════════════════════════════════════

pub(crate) async fn storyclub_pdf_stream(AxumPath(id): AxumPath<String>, headers: HeaderMap) -> Response {
    let issue = match crate::db::storyclub::get_issue(&id) {
        Ok(Some(i)) => i,
        _ => return (StatusCode::NOT_FOUND, "期数不存在").into_response(),
    };
    let p = Path::new(&issue.path);
    if !p.is_file() {
        return (StatusCode::NOT_FOUND, "PDF 文件不存在").into_response();
    }
    // 转发 Range 头，pdf.js 按需分块拉取（大扫描件无需整份传输）
    let mut req = Request::new(Body::empty());
    *req.headers_mut() = headers;
    match ServeFile::new(p.to_path_buf()).oneshot(req).await {
        Ok(resp) => resp.into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("PDF 流失败: {e}"),
        )
            .into_response(),
    }
}

// ══════════════════════════════════════════════════════════
//  图片二进制流（浏览器模式流式加载，替代 base64；均需口令）
// ══════════════════════════════════════════════════════════

/// 二进制图片响应（带 Content-Type 与短缓存，浏览器原生解码）
fn img_response(mime: String, bytes: Vec<u8>) -> Response {
    (
        [
            (axum::http::header::CONTENT_TYPE, mime),
            (
                axum::http::header::CACHE_CONTROL,
                "private, max-age=3600".to_string(),
            ),
        ],
        bytes,
    )
        .into_response()
}

/// 在线程池执行图片字节读取（封面/页面可能触发解码缩放，避免阻塞 tokio worker）
async fn img_bytes_spawn(
    f: impl FnOnce() -> Result<(String, Vec<u8>), String> + Send + 'static,
) -> Response {
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok((mime, bytes))) => img_response(mime, bytes),
        Ok(Err(e)) => err_json(e),
        Err(e) => err_json(format!("线程异常: {e}")),
    }
}

/// GET /api/img/comic-cover/{id}
pub(crate) async fn img_comic_cover(AxumPath(id): AxumPath<String>) -> Response {
    img_bytes_spawn(move || crate::commands::comic_cover_bytes(&id)).await
}

/// GET /api/img/comic-page/{id}/{idx}
pub(crate) async fn img_comic_page(AxumPath((id, idx)): AxumPath<(String, String)>) -> Response {
    let idx = match idx.parse::<usize>() {
        Ok(v) => v,
        Err(_) => return err_json("页码无效".to_string()),
    };
    img_bytes_spawn(move || crate::commands::comic_page_bytes(&id, idx)).await
}

/// GET /api/img/comic-preview/{id}/{idx}
pub(crate) async fn img_comic_preview(AxumPath((id, idx)): AxumPath<(String, String)>) -> Response {
    let idx = match idx.parse::<usize>() {
        Ok(v) => v,
        Err(_) => return err_json("页码无效".to_string()),
    };
    img_bytes_spawn(move || crate::commands::comic_preview_bytes(&id, idx)).await
}

/// GET /api/img/comic-pdf/{id}：PDF 原始字节（Range 分块，pdf.js 按需拉取）
pub(crate) async fn img_comic_pdf(AxumPath(id): AxumPath<String>, headers: HeaderMap) -> Response {
    let comic = match crate::db::comics::get_comic(&id) {
        Ok(Some(c)) => c,
        _ => return (StatusCode::NOT_FOUND, "漫画不存在").into_response(),
    };
    if comic.kind != "pdf" {
        return (StatusCode::BAD_REQUEST, "该条目不是 PDF").into_response();
    }
    let p = Path::new(&comic.root_dir).join(&comic.path);
    if !p.is_file() {
        return (StatusCode::NOT_FOUND, "PDF 文件不存在").into_response();
    }
    // 转发 Range 头，pdf.js 按需分块拉取
    let mut req = Request::new(Body::empty());
    *req.headers_mut() = headers;
    match ServeFile::new(p).oneshot(req).await {
        Ok(resp) => resp.into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("PDF 流失败: {e}"),
        )
            .into_response(),
    }
}

/// GET /api/img/video-cover/{id}
pub(crate) async fn img_video_cover(AxumPath(id): AxumPath<String>) -> Response {
    img_bytes_spawn(move || crate::commands::video_cover_bytes(&id)).await
}

/// GET /api/img/actor/{id}
pub(crate) async fn img_actor(AxumPath(id): AxumPath<String>) -> Response {
    img_bytes_spawn(move || crate::commands::actor_image_bytes(&id)).await
}

/// GET /api/img/novel-cover/{id}
pub(crate) async fn img_novel_cover(AxumPath(id): AxumPath<String>) -> Response {
    img_bytes_spawn(move || crate::commands::novel_cover_bytes(&id)).await
}

/// GET /api/img/story-cover/{id}
pub(crate) async fn img_story_cover(AxumPath(id): AxumPath<String>) -> Response {
    img_bytes_spawn(move || crate::commands::storyclub_cover_bytes(&id)).await
}

/// GET /api/img/story-first/{id}
pub(crate) async fn img_story_first(AxumPath(id): AxumPath<String>) -> Response {
    img_bytes_spawn(move || crate::commands::storyclub_first_page_bytes(&id)).await
}

/// GET /api/img/story-page/{id}/{idx} —— 手机端逐页图片流（idx 0 起）
pub(crate) async fn img_story_page(AxumPath((id, idx)): AxumPath<(String, String)>) -> Response {
    let idx = match idx.parse::<usize>() {
        Ok(v) => v,
        Err(_) => return err_json("页码无效".to_string()),
    };
    img_bytes_spawn(move || crate::commands::storyclub_page_bytes(&id, idx)).await
}
