//! HTTP 服务层——局域网地址与口令鉴权。
//! 原 http.rs 拆分（路由见 http.rs，命令分发见 http_dispatch，流/浏览见 http_stream）。
use super::PORT;
use axum::{
    extract::Request as AxumRequest,
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 口令在 meta 表中的 key
const PASSWORD_KEY: &str = "web_password";

// ══════════════════════════════════════════════════════════
//  局域网地址检测（桌面窗口展示访问链接用）
// ══════════════════════════════════════════════════════════

/// 本机可用的局域网访问地址（默认路由出口 IP + localhost）
pub fn web_urls() -> Vec<String> {
    let mut urls = vec![format!("http://localhost:{PORT}")];
    if let Some(ip) = default_route_ip() {
        let u = format!("http://{ip}:{PORT}");
        if !urls.contains(&u) {
            urls.insert(0, u);
        }
    }
    urls
}

/// 探测默认路由出口 IP（UDP connect 不发包，仅取本机出口地址）
fn default_route_ip() -> Option<String> {
    for target in ["8.8.8.8:53", "223.5.5.5:53", "114.114.114.114:53"] {
        if let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0") {
            if sock.connect(target).is_ok() {
                if let Ok(addr) = sock.local_addr() {
                    return Some(addr.ip().to_string());
                }
            }
        }
    }
    // 兜底：主机名解析出第一个非回环私网 IPv4
    std::env::var("COMPUTERNAME").ok().and_then(|name| {
        use std::net::ToSocketAddrs;
        (name.as_str(), 0)
            .to_socket_addrs()
            .ok()?
            .find_map(|a| match a.ip() {
                std::net::IpAddr::V4(v4) if !v4.is_loopback() && is_private_v4(&v4) => {
                    Some(v4.to_string())
                }
                _ => None,
            })
    })
}

fn is_private_v4(ip: &std::net::Ipv4Addr) -> bool {
    let [a, b, _, _] = ip.octets();
    a == 10 || (a == 172 && (16..=31).contains(&b)) || (a == 192 && b == 168)
}

// ══════════════════════════════════════════════════════════
//  口令鉴权
// ══════════════════════════════════════════════════════════

/// 读取口令（无则生成随机 6 位数字并存库）
pub fn web_password() -> String {
    if let Ok(Some(p)) = crate::db::get_setting(PASSWORD_KEY) {
        if !p.is_empty() {
            return p;
        }
    }
    let seed = format!("{:?}{}", std::time::SystemTime::now(), std::process::id());
    let digest = format!("{:x}", md5::compute(seed.as_bytes()));
    let num = u64::from_str_radix(&digest[..12], 16).unwrap_or(0);
    let code = format!("{:06}", num % 1_000_000);
    let _ = crate::db::set_setting(PASSWORD_KEY, &code);
    log::info!("已生成局域网访问口令: {code}");
    code
}

/// 请求是否携带有效口令（Header X-Auth 或 query token）
fn authed(headers: &HeaderMap, query: &Value) -> bool {
    let pass = web_password();
    let from_header = headers
        .get("x-auth")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if from_header == pass {
        return true;
    }
    query
        .as_object()
        .and_then(|m| m.get("token"))
        .and_then(|v| v.as_str())
        .map(|t| t == pass)
        .unwrap_or(false)
}

/// 口令中间件：/api 下除 auth 开外的路径均要求口令
pub(crate) async fn require_auth(req: AxumRequest, next: Next) -> Response {
    let path = req.uri().path().to_string();
    if path.starts_with("/api/auth/") {
        return next.run(req).await;
    }
    let headers = req.headers().clone();
    let query = req
        .uri()
        .query()
        .map(|q| serde_urlencoded_parse(q))
        .unwrap_or(Value::Null);
    if authed(&headers, &query) {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "ok": false, "error": "未授权：请先输入访问口令" })),
        )
            .into_response()
    }
}

/// 简易 query 解析（仅提取 token，避免引入 serde_urlencoded 依赖）
fn serde_urlencoded_parse(q: &str) -> Value {
    let mut obj = serde_json::Map::new();
    for pair in q.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            obj.insert(k.to_string(), Value::String(v.to_string()));
        }
    }
    Value::Object(obj)
}

#[derive(Serialize)]
struct AuthStatus {
    enabled: bool,
    authed: bool,
}

/// GET /api/auth/status → 是否启用口令 / 本次请求是否已认证
pub(crate) async fn auth_status(headers: HeaderMap) -> Response {
    let enabled = !web_password().is_empty();
    let authed = authed(&headers, &Value::Null);
    Json(json!(AuthStatus { enabled, authed })).into_response()
}

#[derive(Deserialize)]
pub(crate) struct LoginBody {
    password: String,
}

/// POST /api/auth/login → 校验口令
pub(crate) async fn auth_login(Json(body): Json<LoginBody>) -> Response {
    if body.password == web_password() {
        Json(json!({ "ok": true, "token": body.password })).into_response()
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "ok": false, "error": "口令错误" })),
        )
            .into_response()
    }
}

/// GET /api/auth/password → 读取当前口令（需已认证）
pub(crate) async fn auth_get_password() -> Response {
    Json(json!({ "password": web_password() })).into_response()
}

#[derive(Deserialize)]
pub(crate) struct SetPasswordBody {
    password: String,
}

/// POST /api/auth/password → 修改口令（需已认证；桌面设置页使用）
pub(crate) async fn auth_set_password(Json(body): Json<SetPasswordBody>) -> Response {
    let p = body.password.trim().to_string();
    if p.len() < 4 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": "口令至少 4 位" })),
        )
            .into_response();
    }
    match crate::db::set_setting(PASSWORD_KEY, &p) {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "ok": false, "error": e })),
        )
            .into_response(),
    }
}
