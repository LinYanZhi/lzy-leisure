//! 小说（EPUB）命令。原 commands.rs 拆分，外部路径 `commands::xxx` 不变。
use super::{data_url, mime_from_ext};
use crate::db::novels as novel_db;
use std::path::Path;

// ══════════════════════════════════════════════════════════
//  小说（EPUB：单本即一部书，只读解析，绝不修改原文件）
// ══════════════════════════════════════════════════════════

/// 递归收集目录下全部 epub 文件（跳过隐藏目录）
fn find_epubs(dir: &Path) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    let mut stack: Vec<std::path::PathBuf> = vec![dir.to_path_buf()];
    while let Some(cur) = stack.pop() {
        let entries = match std::fs::read_dir(&cur) {
            Ok(e) => e,
            Err(e) if cur == dir => {
                return Err(format!("读取目录失败 {}: {e}", cur.display()));
            }
            Err(e) => {
                log::warn!("跳过无法读取的目录 {}: {e}", cur.display());
                continue;
            }
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                stack.push(p);
            } else if p.is_file() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.to_ascii_lowercase().ends_with(".epub") {
                    out.push(p.to_string_lossy().to_string());
                }
            }
        }
    }
    out.sort();
    Ok(out)
}

/// 解析 EPUB → 登记书籍（幂等：按 path 匹配，不覆盖阅读进度）
fn register_novel_impl(path: &Path, root_dir: &str) -> Result<novel_db::Novel, String> {
    let meta = crate::novel_epub::parse_epub(path)?;
    let chapters: Vec<novel_db::NovelChapter> = meta
        .chapters
        .iter()
        .enumerate()
        .map(|(i, c)| novel_db::NovelChapter {
            id: i as i64,
            title: c.title.clone(),
            href: c.href.clone(),
        })
        .collect();
    let title = if meta.title.trim().is_empty() {
        path.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    } else {
        meta.title.trim().to_string()
    };
    let novel = novel_db::Novel {
        id: crate::db::new_video_id(),
        title,
        author: meta.author.trim().to_string(),
        path: path.to_string_lossy().to_string(),
        cover_path: String::new(),
        reading_pos: 0,
        chapter: String::new(),
        root_dir: root_dir.to_string(),
        chapter_count: chapters.len() as i64,
        sort_order: 0,
        created_at: String::new(),
        updated_at: String::new(),
        chapters,
    };
    novel_db::upsert_novel(&novel)?;
    Ok(novel)
}

/// 路径相等（大小写不敏感，忽略分隔符写法差异）
fn novel_paths_equal(a: &str, b: &str) -> bool {
    let parts = |p: &str| {
        Path::new(p)
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
            .collect::<Vec<String>>()
    };
    parts(a) == parts(b)
}

/// 添加小说目录：登记路径 + 扫描该目录（含子目录）下全部 EPUB。
/// 先登记路径再扫描——即使扫描失败（权限/IO 等），路径也会出现在源管理中，可单独重扫。
#[tauri::command]
pub(crate) async fn add_novel_root(path: String) -> Result<String, String> {
    if !Path::new(&path).is_dir() {
        return Err(format!("目录不存在: {path}"));
    }
    let roots = novel_db::get_novel_roots()?;
    if roots.iter().any(|r| novel_paths_equal(&r.path, &path)) {
        return Err(format!("「{path}」已在小说目录列表中，无需重复导入"));
    }
    let path2 = path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let name = Path::new(&path2)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path2.clone());
        novel_db::add_novel_root(&path2, &name)?;
        let (new_count, removed) = scan_novel_root_impl(Path::new(&path2))?;
        crate::events::emit(
            "novel-scan-done",
            serde_json::json!({
                "root_dir": path2,
                "new_count": new_count,
                "removed": removed,
            }),
        );
        Ok::<String, String>(format!("扫描完成：新增 {new_count} 本，移除 {removed} 本"))
    })
    .await
    .map_err(|e| format!("扫描任务失败: {e}"))?
}

/// 重新扫描单个小说目录（登记新增，移除磁盘已不存在的书）
#[tauri::command]
pub(crate) async fn rescan_novel_root(path: String) -> Result<String, String> {
    let path2 = path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if !Path::new(&path2).is_dir() {
            return Err(format!("目录不存在: {path2}"));
        }
        let (new_count, removed) = scan_novel_root_impl(Path::new(&path2))?;
        crate::events::emit(
            "novel-scan-done",
            serde_json::json!({
                "root_dir": path2,
                "new_count": new_count,
                "removed": removed,
            }),
        );
        Ok::<String, String>(format!("重新扫描完成：新增 {new_count} 本，移除 {removed} 本"))
    })
    .await
    .map_err(|e| format!("扫描任务失败: {e}"))?
}

/// 共享扫描实现：登记目录下全部 EPUB，移除磁盘上已不存在的书
fn scan_novel_root_impl(dir: &Path) -> Result<(i64, i64), String> {
    let root = dir.to_string_lossy().to_string();
    let files = find_epubs(dir)?;
    let mut new_count = 0i64;
    for f in &files {
        if novel_db::get_novel_by_path(f)?.is_some() {
            continue;
        }
        match register_novel_impl(Path::new(f), &root) {
            Ok(_) => new_count += 1,
            Err(e) => log::warn!("解析失败（跳过）{f}: {e}"),
        }
    }
    let removed = novel_db::delete_novels_not_in(&root, &files)? as i64;
    Ok((new_count, removed))
}

#[tauri::command]
pub(crate) fn remove_novel_root(path: String) -> Result<(), String> {
    novel_db::remove_novel_root(&path)
}

#[tauri::command]
pub(crate) fn get_novel_roots() -> Result<Vec<novel_db::NovelRootDir>, String> {
    novel_db::get_novel_roots()
}

/// 单文件导入 EPUB（幂等：已导入则直接返回）
#[tauri::command]
pub(crate) fn add_novel(path: String) -> Result<String, String> {
    let p = Path::new(&path);
    if !p.is_file() {
        return Err(format!("文件不存在: {path}"));
    }
    if !path.to_ascii_lowercase().ends_with(".epub") {
        return Err("仅支持 EPUB 文件".to_string());
    }
    if novel_db::get_novel_by_path(&path)?.is_some() {
        return Ok("ok".to_string());
    }
    register_novel_impl(p, "")?;
    Ok("ok".to_string())
}

#[tauri::command]
pub(crate) fn list_novels() -> Result<Vec<novel_db::Novel>, String> {
    novel_db::list_novels()
}

#[tauri::command]
pub(crate) fn get_novel(novel_id: String) -> Result<Option<novel_db::Novel>, String> {
    novel_db::get_novel(&novel_id)
}

#[tauri::command]
pub(crate) fn delete_novel(novel_id: String) -> Result<(), String> {
    novel_db::delete_novel(&novel_id)
}

/// 改名（只改库记录，不碰文件）
#[tauri::command]
pub(crate) fn rename_novel(novel_id: String, title: String) -> Result<(), String> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err("标题不能为空".to_string());
    }
    novel_db::rename_novel(&novel_id, &title)
}

/// 小说封面字节（HTTP 流式接口与 data URL 命令共用；从 EPUB 内嵌封面提取，缓存到应用数据目录）
pub(crate) fn novel_cover_bytes(novel_id: &str) -> Result<(String, Vec<u8>), String> {
    let novel = novel_db::get_novel(novel_id)?.ok_or_else(|| "书籍不存在".to_string())?;
    // 1. 已有封面缓存
    if !novel.cover_path.is_empty() {
        let p = crate::app_data_dir().join(&novel.cover_path);
        if p.is_file() {
            if let Ok(bytes) = std::fs::read(&p) {
                return Ok((mime_from_ext(&novel.cover_path), bytes));
            }
        }
    }
    // 2. 从 EPUB 内嵌封面提取并缓存
    let meta = crate::novel_epub::parse_epub(Path::new(&novel.path))?;
    let cover = meta.cover.ok_or_else(|| "该 EPUB 没有内嵌封面".to_string())?;
    let ext = if cover.mime.contains("png") { "png" } else { "jpg" };
    let rel = format!("covers/{novel_id}.{ext}");
    let target = crate::app_data_dir().join(&rel);
    std::fs::write(&target, &cover.bytes).map_err(|e| format!("保存封面失败: {e}"))?;
    novel_db::set_novel_cover(novel_id, &rel)?;
    Ok((cover.mime, cover.bytes))
}

/// 获取小说封面 data URL（从 EPUB 内嵌封面提取，缓存到应用数据目录；只读原文件）
#[tauri::command]
pub(crate) fn get_novel_cover_data_url(novel_id: String) -> Result<String, String> {
    let (mime, bytes) = novel_cover_bytes(&novel_id)?;
    Ok(data_url(&mime, &bytes))
}

/// 读取章节纯文本（只读原文件）
#[tauri::command]
pub(crate) fn get_novel_chapter_content(novel_id: String, chapter_index: i64) -> Result<String, String> {
    let novel = novel_db::get_novel(&novel_id)?.ok_or_else(|| "书籍不存在".to_string())?;
    let ch = novel
        .chapters
        .get(chapter_index as usize)
        .ok_or_else(|| "章节不存在".to_string())?;
    crate::novel_epub::read_chapter_text(Path::new(&novel.path), &ch.href)
}

/// 保存阅读进度（chapter 为章节索引，pos 为章节内字符偏移；仅存库）
#[tauri::command]
pub(crate) fn set_novel_progress(novel_id: String, chapter: i64, pos: i64) -> Result<(), String> {
    novel_db::set_novel_progress(&novel_id, &chapter.to_string(), pos)
}

/// 打开小说所在目录（只读）
#[tauri::command]
pub(crate) fn open_novel_folder(novel_id: String) -> Result<String, String> {
    let novel = novel_db::get_novel(&novel_id)?.ok_or_else(|| "书籍不存在".to_string())?;
    let p = Path::new(&novel.path);
    let dir = p.parent().unwrap_or(p);
    if !dir.exists() {
        return Err(format!("目录不存在: {}", dir.display()));
    }
    opener::open(dir).map_err(|e| format!("打开目录失败: {e}"))?;
    Ok(dir.to_string_lossy().to_string())
}
