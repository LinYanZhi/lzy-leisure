//! 全局事件总线：后端 → 桌面窗口（Tauri emit）与浏览器（HTTP SSE）共用的广播通道。
//!
//! 命令层调用 events::emit 广播事件，不再直接依赖 tauri AppHandle：
//!   - 桌面：bus 内同时转发给所有 webview 窗口（与旧行为一致）
//!   - 浏览器：SSE 端点（http::api_events）订阅同一广播通道

use std::sync::OnceLock;
use tokio::sync::broadcast;

/// 通道容量（事件是低频通知，少量即可）
const CHANNEL: usize = 64;

static TX: OnceLock<broadcast::Sender<String>> = OnceLock::new();

fn tx() -> &'static broadcast::Sender<String> {
    TX.get_or_init(|| broadcast::channel(CHANNEL).0)
}

/// 广播事件（载荷为完整 JSON：{ "event": "...", "payload": {...} }），
/// 同时转发给桌面窗口（tauri emit）。
pub fn emit(name: &str, data: serde_json::Value) {
    let payload =
        serde_json::json!({ "event": name, "payload": data }).to_string();
    let _ = tx().send(payload);
    if let Some(app) = crate::APP.get() {
        use tauri::Emitter;
        let _ = app.emit(name, data);
    }
}

/// 订阅事件流（SSE 连接用；返回的 Receiver 会错过历史事件，只收之后的广播）
pub fn subscribe() -> broadcast::Receiver<String> {
    tx().subscribe()
}
