//! 漫画目录扫描器——index.json 处理层。
//! 原 scanner.rs 拆分（文件工具见 scanner_fs，扫描编排见 scanner.rs）：
//! 角色推断、条目收集/同步、标题覆盖与完结状态读写。
use super::scanner_fs::{is_archive, is_hidden_dir, is_meta_file, is_pdf};
use super::any_child_has_index;
use crate::db;
use std::collections::HashMap;
use std::path::Path;

// ══════════════════════════════════════════════════════════
//  index.json 处理
// ══════════════════════════════════════════════════════════

/// index.json 解析结果
pub struct IndexMeta {
    /// 章节目录名 → 标题覆盖
    pub title_map: HashMap<String, String>,
    /// 章节目录名 → 排序号
    pub order_map: HashMap<String, i64>,
    /// 根级完结标志
    pub completed: Option<bool>,
    /// 目录角色声明（collection / comic / single，缺省则按结构推断）
    pub role: Option<String>,
}

/// 读取 index.json（不存在或非法返回 None）
pub fn load_index_meta(root_dir: &str) -> Option<IndexMeta> {
    let index_path = Path::new(root_dir).join("index.json");
    if !index_path.is_file() {
        return None;
    }
    let content = std::fs::read_to_string(&index_path).ok()?;
    let data: serde_json::Value = serde_json::from_str(&content).ok()?;
    let mut meta = IndexMeta {
        title_map: HashMap::new(),
        order_map: HashMap::new(),
        completed: None,
        role: None,
    };
    if let Some(entries) = data.get("entries").and_then(|e| e.as_array()) {
        for entry in entries {
            let dir_name = entry
                .get("dir_name")
                .and_then(|d| d.as_str())
                .unwrap_or_default()
                .to_string();
            if dir_name.is_empty() {
                continue;
            }
            if let Some(title) = entry.get("title").and_then(|t| t.as_str()) {
                meta.title_map.insert(dir_name.clone(), title.to_string());
            }
            if let Some(order) = entry.get("order").and_then(|o| o.as_i64()) {
                meta.order_map.insert(dir_name, order);
            }
        }
    }
    meta.completed = data.get("completed").and_then(|v| v.as_bool());
    meta.role = data
        .get("role")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Some(meta)
}

/// 读取 index.json，返回 (title_map, order_map)（兼容旧调用方）
pub fn load_index_json(root_dir: &str) -> Option<(HashMap<String, String>, HashMap<String, i64>)> {
    load_index_meta(root_dir).map(|m| (m.title_map, m.order_map))
}

// ══════════════════════════════════════════════════════════
//  目录角色（index.json 的 role 字段）
// ══════════════════════════════════════════════════════════

/// 父级集合：子项是独立漫画（如根目录、装多部漫画的文件夹）
pub const ROLE_COLLECTION: &str = "collection";
/// 漫画本体：子项是章节（系列漫画）
pub const ROLE_COMIC: &str = "comic";
/// 单本漫画：无子项，目录直接含图片/PDF
pub const ROLE_SINGLE: &str = "single";

/// 按目录结构推断角色（无 role 声明时的回退）：
/// 存在含 index.json 的子目录 → 子项是独立漫画 → collection；
/// 否则子项视为章节 → comic。
pub(crate) fn infer_role(dir: &Path) -> &'static str {
    if any_child_has_index(dir) {
        ROLE_COLLECTION
    } else {
        ROLE_COMIC
    }
}

/// 旧 index.json 缺 role 声明时按推断值补写（一次性迁移）。
/// 用户手写或已存在的 role 不被覆盖。
pub(crate) fn write_role_if_missing(root_dir: &str, role: &str) {
    let index_path = Path::new(root_dir).join("index.json");
    if !index_path.is_file() {
        return;
    }
    let Ok(content) = std::fs::read_to_string(&index_path) else {
        return;
    };
    let mut data: serde_json::Value = match serde_json::from_str::<serde_json::Value>(&content) {
        Ok(v) if v.is_object() => v,
        _ => return,
    };
    if data.get("role").is_some() {
        return;
    }
    data["role"] = serde_json::json!(role);
    if let Ok(out) = serde_json::to_string_pretty(&data) {
        let _ = std::fs::write(&index_path, out);
    }
}

/// 目录名 → 友好的章节标题
fn entry_to_display_name(entry: &str) -> String {
    if entry.contains("第") && entry.chars().any(|c| c.is_ascii_digit()) {
        return entry.to_string();
    }
    // ch01 / chapter1 / vol01 / 001
    let lower = entry.to_lowercase();
    if let Some(rest) = lower
        .strip_prefix("chapter")
        .or_else(|| lower.strip_prefix("ch"))
    {
        let rest = rest.trim_start_matches(['_', '.', ' ', '-']);
        if let Ok(n) = rest.parse::<i64>() {
            return format!("第{n}章");
        }
    }
    if let Some(rest) = lower.strip_prefix("vol").or_else(|| lower.strip_prefix("volume")) {
        let rest = rest.trim_start_matches(['_', '.', ' ', '-']);
        if let Ok(n) = rest.parse::<i64>() {
            return format!("第{n}卷");
        }
    }
    if let Ok(n) = entry.trim_start_matches('0').parse::<i64>() {
        return format!("第{n}话");
    }
    if entry.chars().any(|c| c.is_ascii_digit()) && !entry.chars().any(|c| c.is_ascii_alphabetic()) {
        if let Ok(n) = entry.trim_start_matches('0').parse::<i64>() {
            return format!("第{n}话");
        }
    }
    if entry.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)) {
        return entry.to_string();
    }
    entry
        .split(['_', '.', ' ', '-'])
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// 收集根目录下的章节条目（子目录 + 压缩包，自然排序）
pub(crate) fn collect_chapter_entries(root_dir: &str) -> Vec<String> {
    let mut entries: Vec<String> = Vec::new();
    if let Ok(read_dir) = std::fs::read_dir(root_dir) {
        for entry in read_dir.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if is_hidden_dir(&name) || is_meta_file(&name) {
                continue;
            }
            if entry.path().is_dir() || is_archive(&name) || is_pdf(&name) {
                entries.push(name);
            }
        }
    }
    entries.sort_by_key(|e| db::natural_key(e));
    entries
}

/// 若无 index.json 则自动生成（默认标题 + 自然序 + 推断的 role）。
/// 单本目录（无章节条目）生成空 entries + role=single，同样作为「漫画单元」标识。
pub fn ensure_index_json(root_dir: &str) {
    let index_path = Path::new(root_dir).join("index.json");
    if index_path.exists() {
        return;
    }
    let chapter_entries = collect_chapter_entries(root_dir);
    if chapter_entries.is_empty() {
        let data = serde_json::json!({ "entries": [], "role": ROLE_SINGLE });
        if let Ok(content) = serde_json::to_string_pretty(&data) {
            let _ = std::fs::write(&index_path, content);
        }
        return;
    }
    let role = infer_role(Path::new(root_dir));
    let entries: Vec<serde_json::Value> = chapter_entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            serde_json::json!({
                "order": i + 1,
                "title": entry_to_display_name(e),
                "dir_name": e,
            })
        })
        .collect();
    let data = serde_json::json!({ "entries": entries, "role": role });
    if let Ok(content) = serde_json::to_string_pretty(&data) {
        let _ = std::fs::write(&index_path, content);
    }
}

/// 同步 index.json 与磁盘状态：
/// 移除已不存在的条目、追加新增条目、按自然序重编号，
/// 并保留已编辑的标题、根级 completed 标志与 role 声明（缺省时补写推断值）。
/// 返回是否有变更。
pub fn sync_index_json(root_dir: &str) -> bool {
    let index_path = Path::new(root_dir).join("index.json");
    let disk_dirs = collect_chapter_entries(root_dir);
    if disk_dirs.is_empty() {
        return false;
    }

    // 读取现有 index.json（保留 completed 标志、role 声明与已保存标题）
    let mut completed: Option<bool> = None;
    let mut role: Option<String> = None;
    let mut title_map: HashMap<String, String> = HashMap::new();
    if index_path.is_file() {
        if let Ok(content) = std::fs::read_to_string(&index_path) {
            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(c) = data.get("completed").and_then(|v| v.as_bool()) {
                    completed = Some(c);
                }
                role = data
                    .get("role")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                if let Some(entries) = data.get("entries").and_then(|e| e.as_array()) {
                    for entry in entries {
                        let dir_name = entry
                            .get("dir_name")
                            .and_then(|d| d.as_str())
                            .unwrap_or_default()
                            .to_string();
                        if disk_dirs.contains(&dir_name) {
                            if let Some(title) = entry.get("title").and_then(|t| t.as_str()) {
                                title_map.insert(dir_name, title.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    // role 缺省（旧文件）时按结构推断补写；用户手写的值优先保留
    let role = role.unwrap_or_else(|| infer_role(Path::new(root_dir)).to_string());

    // 合并：按磁盘自然序重建条目
    let merged: Vec<serde_json::Value> = disk_dirs
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let title = title_map
                .get(e)
                .cloned()
                .unwrap_or_else(|| entry_to_display_name(e));
            serde_json::json!({
                "order": i + 1,
                "title": title,
                "dir_name": e,
            })
        })
        .collect();

    // 与现有条目对比，无变化则跳过写入
    let mut unchanged = true;
    if let Ok(content) = std::fs::read_to_string(&index_path) {
        if let Ok(data) = serde_json::from_str::<serde_json::Value>(&content) {
            let existing = data.get("entries").and_then(|e| e.as_array()).cloned().unwrap_or_default();
            let existing_role = data.get("role").and_then(|v| v.as_str()).unwrap_or_default();
            unchanged = existing == merged
                && completed == data.get("completed").and_then(|v| v.as_bool())
                && existing_role == role;
        }
    }
    if unchanged {
        return false;
    }

    let mut output = serde_json::json!({ "entries": merged, "role": role });
    if let Some(c) = completed {
        output["completed"] = serde_json::json!(c);
    }
    if let Ok(content) = serde_json::to_string_pretty(&output) {
        let _ = std::fs::write(&index_path, content);
        return true;
    }
    false
}

/// 读取系列完结标志（index.json 根级 completed）
pub fn get_completed_status(root_dir: &str) -> Option<bool> {
    let index_path = Path::new(root_dir).join("index.json");
    if !index_path.is_file() {
        return None;
    }
    let content = std::fs::read_to_string(&index_path).ok()?;
    let data: serde_json::Value = serde_json::from_str(&content).ok()?;
    data.get("completed").and_then(|v| v.as_bool())
}

/// 更新 index.json 中某章节条目的标题（dir_name = 章节相对路径的文件名）。
/// 标题权威统一为 index.json：重命名章节时同步写回，避免被覆盖导致重命名失效。
/// index.json 不存在或条目缺失时静默跳过（目录元数据尽力维护，不报错）。
pub fn set_entry_title(root_dir: &str, ch_rel_path: &str, title: &str) {
    let dir_name = Path::new(ch_rel_path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if dir_name.is_empty() {
        return;
    }
    let index_path = Path::new(root_dir).join("index.json");
    if !index_path.is_file() {
        return;
    }
    let Ok(content) = std::fs::read_to_string(&index_path) else {
        return;
    };
    let Ok(mut data) = serde_json::from_str::<serde_json::Value>(&content) else {
        return;
    };
    if !data.is_object() {
        return;
    }
    let Some(entries) = data.get_mut("entries").and_then(|e| e.as_array_mut()) else {
        return;
    };
    let mut changed = false;
    for entry in entries {
        if entry.get("dir_name").and_then(|d| d.as_str()) == Some(dir_name.as_str()) {
            entry["title"] = serde_json::json!(title);
            changed = true;
            break;
        }
    }
    if changed {
        if let Ok(out) = serde_json::to_string_pretty(&data) {
            let _ = std::fs::write(&index_path, out);
        }
    }
}

/// 写入系列完结标志（index.json 根级 completed）
pub fn set_completed_status(root_dir: &str, completed: bool) -> Result<(), String> {
    let index_path = Path::new(root_dir).join("index.json");
    let mut data: serde_json::Value = if index_path.is_file() {
        std::fs::read_to_string(&index_path)
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_else(|| serde_json::json!({ "entries": [] }))
    } else {
        serde_json::json!({ "entries": [] })
    };
    if !data.is_object() {
        data = serde_json::json!({ "entries": [] });
    }
    data["completed"] = serde_json::json!(completed);
    let content = serde_json::to_string_pretty(&data).map_err(|e| format!("序列化失败: {e}"))?;
    std::fs::write(&index_path, content).map_err(|e| format!("写入 index.json 失败: {e}"))?;
    Ok(())
}
