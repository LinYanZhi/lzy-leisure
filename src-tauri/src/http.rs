//! HTTP 服务层：本机/局域网浏览器访问（口令保护）
//!
//! 与桌面窗口共用同一套命令逻辑（commands 模块，见 lib.rs 说明）。
//! 路由：
//!   POST /api/{command}            命令分发（与 tauri command 同名，JSON 参数，snake_case）
//!   GET  /api/auth/status          鉴权状态（是否启用口令 / 当前请求是否已认证）
//!   POST /api/auth/login           口令登录 { password }
//!   GET  /api/auth/password        读取口令（需已认证，桌面设置用）
//!   POST /api/auth/password        修改口令 { password }（需已认证）
//!   GET  /api/events               SSE 事件推送（scan-done），query 需带 token
//!   GET  /api/fs/list?path=        目录浏览（浏览器模式选目录，需已认证）
//!   GET  /api/video/stream/{id}    视频流（HTTP Range，需已认证）
//!   其它 GET → 前端静态资源（生产托管 ../dist，SPA fallback 到 index.html）
//!
//! 按职责拆分为子模块（外部 crate::http::* 路径不变）：
//!   - `http_auth.rs`    局域网地址检测与口令鉴权
//!   - `http_dispatch.rs` POST /api/{command} 命令分发
//!   - `http_stream.rs`  SSE 事件、目录浏览、视频/PDF Range 流、图片二进制流
pub(crate) mod http_auth;
pub(crate) mod http_dispatch;
pub(crate) mod http_stream;

use axum::{
    extract::Request as AxumRequest,
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use std::path::Path;
use tower::ServiceExt;
use tower_http::services::ServeFile;

use http_auth::{
    auth_get_password, auth_login, auth_set_password, auth_status, require_auth,
};
use http_dispatch::api_command;
use http_stream::{
    api_events, fs_list, img_actor, img_comic_cover, img_comic_page, img_comic_pdf,
    img_comic_preview, img_novel_cover, img_story_cover, img_story_first, img_story_page,
    img_video_cover, storyclub_pdf_stream, video_stream,
};

// 重导出：保持 crate::http::* 外部路径不变
pub use http_auth::{web_password, web_urls};

/// 服务端口（固定，浏览器/局域网访问用）
pub const PORT: u16 = 5184;

/// 启动 HTTP 服务（setup 阶段调用；绑定 0.0.0.0 支持局域网 IP 访问）
pub fn start() {
    tauri::async_runtime::spawn(async move {
        let app = build_router();
        let addr = format!("0.0.0.0:{PORT}");
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(listener) => {
                log::info!("局域网服务已启动: http://{addr}（浏览器/局域网访问，口令见设置）");
                if let Err(e) = axum::serve(listener, app).await {
                    log::error!("HTTP 服务异常退出: {e}");
                }
            }
            Err(e) => log::error!("绑定 {addr} 失败（HTTP 服务未启动）: {e}"),
        }
    });
}

fn build_router() -> Router {
    let mut router = Router::new()
        // 鉴权状态 / 登录不要求口令
        .route("/api/auth/status", get(auth_status))
        .route("/api/auth/login", post(auth_login))
        .route(
            "/api/auth/password",
            get(auth_get_password).post(auth_set_password),
        )
        // 业务命令 / 事件 / 目录浏览 / 视频流（均需口令）
        .route("/api/:command", post(api_command))
        .route("/api/events", get(api_events))
        .route("/api/fs/list", get(fs_list))
        .route("/api/video/stream/:id", get(video_stream))
        .route("/api/storyclub/pdf/:id", get(storyclub_pdf_stream))
        // 图片二进制流（浏览器模式流式加载，替代 base64；均需口令）
        .route("/api/img/comic-cover/:id", get(img_comic_cover))
        .route("/api/img/comic-page/:id/:idx", get(img_comic_page))
        .route("/api/img/comic-preview/:id/:idx", get(img_comic_preview))
        .route("/api/img/comic-pdf/:id", get(img_comic_pdf))
        .route("/api/img/video-cover/:id", get(img_video_cover))
        .route("/api/img/actor/:id", get(img_actor))
        .route("/api/img/novel-cover/:id", get(img_novel_cover))
        .route("/api/img/story-cover/:id", get(img_story_cover))
        .route("/api/img/story-first/:id", get(img_story_first))
        .route("/api/img/story-page/:id/:idx", get(img_story_page))
        .layer(middleware::from_fn(require_auth));

    // 生产模式：托管前端构建产物（../dist），SPA fallback 到 index.html
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../dist");
    if dist.is_dir() {
        router = router.fallback(spa_fallback);
    } else {
        log::warn!("未找到前端构建产物（{}），静态页面不可用", dist.display());
    }
    router
}

/// SPA fallback：命中 dist 下真实文件则原样返回，否则回退 index.html（返回 200）
async fn spa_fallback(req: AxumRequest) -> Response {
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../dist");
    let rel = req.uri().path().trim_start_matches('/');
    let target = if rel.is_empty() {
        dist.join("index.html")
    } else {
        let cand = dist.join(rel);
        if cand.is_file() {
            cand
        } else {
            dist.join("index.html")
        }
    };
    match ServeFile::new(target).oneshot(req).await {
        Ok(resp) => resp.into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("静态资源读取失败: {e}"),
        )
            .into_response(),
    }
}
