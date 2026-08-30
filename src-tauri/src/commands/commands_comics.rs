//! 漫画书架命令（根目录/查询/封面/进度）。原 commands.rs 拆分，外部路径 `commands::xxx` 不变。
use super::{data_url, decode_data_url};
use crate::db::comics as comic_db;
use crate::{covers, scanner};
use base64::Engine as _;
use image::GenericImageView;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ══════════════════════════════════════════════════════════
//  漫画书架
// ══════════════════════════════════════════════════════════

/// 页面信息（阅读器按索引加载）
#[derive(Serialize)]
pub struct PageInfo {
    pub index: usize,
    pub name: String,
    pub mime: String,
}

// ── 根目录管理 ──

/// 路径相等（大小写不敏感，忽略分隔符写法差异）
fn paths_equal(a: &str, b: &str) -> bool {
    let parts = |p: &str| {
        Path::new(p)
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
            .collect::<Vec<String>>()
    };
    parts(a) == parts(b)
}

/// 去重：仅拒绝完全相同路径的重复导入。
/// 包含关系（父目录 ⊇ 子目录）不再自动移除/拒绝：扫描器对超出
/// 「根 → 漫画本体」两层的合集子目录自动跳过（scanner::scan_collection），
/// 宽窄根可安全共存，内容不会重复索引，也避免移除窄根导致阅读进度等用户数据丢失。
fn dedupe_root_dir(new_path: &str) -> Result<(), String> {
    let roots = comic_db::get_root_dirs()?;
    if roots.iter().any(|r| paths_equal(&r.path, new_path)) {
        return Err(format!("「{new_path}」已在根目录列表中，无需重复导入"));
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn add_root_dir(path: String) -> Result<(), String> {
    if !Path::new(&path).is_dir() {
        return Err(format!("目录不存在: {path}"));
    }
    dedupe_root_dir(&path)?;
    comic_db::add_root_dir(&path)
}

#[tauri::command]
pub(crate) fn remove_root_dir(path: String) -> Result<(), String> {
    comic_db::remove_root_dir(&path)
}

#[tauri::command]
pub(crate) fn get_root_dirs() -> Result<Vec<comic_db::RootDir>, String> {
    comic_db::get_root_dirs()
}

/// 扫描漫画根目录（后台线程执行，完成后 emit scan-done）
#[tauri::command]
pub(crate) async fn scan_root_dir(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        // 这里不再调用 dedupe_root_dir：扫描器自身保证内容不重复索引
        // （超层合集子目录跳过，见 scanner::scan_collection），且 rescan
        // 对已存在根重新扫描不应被「相同路径」拦截。
        // 扫描前记录 cover_path，用于封面缓存失效
        let old_covers = comic_db::load_root_cover_paths(&path).unwrap_or_default();

        let comics = scanner::scan_directory(&path)?;
        comic_db::replace_root_comics(&path, &comics)?;
        comic_db::add_root_dir(&path)?;

        // cover.* 变化 → 删除该漫画封面缓存，下次请求重新生成。
        // 仅当该漫画在旧记录中存在（含 cover_path 为空）且值确实变化才删，
        // 避免「移除根目录后重扫」（旧记录已清空）误删仍有效的封面缓存。
        for comic in &comics {
            if !comic.cover_path.is_empty() {
                if let Some(old) = old_covers.get(&comic.id) {
                    if old != &comic.cover_path {
                        // cover.* 变化 → 删除本体封面库中该条目，下次请求重新生成
                        if let Some(bucket) = crate::covers_db::bucket_dir(comic) {
                            let key = crate::covers_db::rel_key_of(comic, &bucket);
                            let _ = crate::covers_db::delete_cover(&bucket, &key);
                        }
                    }
                }
            }
        }
        // 清理各漫画本体外部封面库中已不存在的章节条目（章节被删除/移动后防残留）
        crate::covers_db::purge_after_scan(&comics);

        let top_count = comics.iter().filter(|c| !c.is_container).count();
        let chapter_count = comics.len().saturating_sub(top_count);
        crate::events::emit(
            "scan-done",
            serde_json::json!({
                "root_dir": path,
                "comics": top_count,
                "chapters": chapter_count,
            }),
        );
        Ok::<String, String>(format!("扫描完成：{top_count} 部漫画，{chapter_count} 个章节"))
    })
    .await
    .map_err(|e| format!("扫描任务失败: {e}"))?
}

/// 重新扫描单个漫画（以漫画为基本单位，不动同根目录其他漫画）。
/// 重扫后 id / 进度 / 排序保持稳定（generate_id 基于 root_dir + 相对路径）。
#[tauri::command]
pub(crate) async fn rescan_comic(comic_id: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let comic = comic_db::get_comic(&comic_id)?.ok_or("漫画不存在")?;
        let root_dir = comic.root_dir.clone();
        // 以漫画本体路径为扫描单位
        let abs = if comic.path == "." {
            root_dir.clone()
        } else {
            Path::new(&root_dir).join(&comic.path).to_string_lossy().to_string()
        };
        let mut scanned = scanner::scan_directory(&abs)?;

        // 扫描器以 abs 为根返回相对路径；改为相对真实 root_dir，并重映射稳定 id
        let prefix = if comic.path == "." {
            String::new()
        } else {
            format!("{}/", comic.path.trim_end_matches('/'))
        };
        let mut id_map: HashMap<String, String> = HashMap::new();
        for c in &mut scanned {
            let old = c.id.clone();
            let rel = if prefix.is_empty() {
                if c.path.is_empty() || c.path == "." {
                    ".".to_string()
                } else {
                    c.path.clone()
                }
            } else {
                if c.path.is_empty() || c.path == "." {
                    comic.path.trim_end_matches('/').to_string()
                } else {
                    format!("{prefix}{}", c.path.trim_start_matches('/'))
                }
            };
            c.path = rel.clone();
            let new = crate::db::generate_id(&root_dir, &rel);
            id_map.insert(old, new.clone());
            c.id = new;
        }
        for c in &mut scanned {
            if !c.series_id.is_empty() {
                if let Some(n) = id_map.get(&c.series_id) {
                    c.series_id = n.clone();
                }
            }
        }

        comic_db::replace_comic(&root_dir, &comic_id, &scanned)?;

        // cover.* 变化 → 删除封面缓存，下次请求重新生成
        let old_covers = comic_db::load_root_cover_paths(&root_dir).unwrap_or_default();
        for c in &scanned {
            if !c.cover_path.is_empty() {
                if let Some(old) = old_covers.get(&c.id) {
                    if old != &c.cover_path {
                        if let Some(bucket) = crate::covers_db::bucket_dir(c) {
                            let key = crate::covers_db::rel_key_of(c, &bucket);
                            let _ = crate::covers_db::delete_cover(&bucket, &key);
                        }
                    }
                }
            }
        }
        crate::covers_db::purge_after_scan(&scanned);

        let top_count = scanned.iter().filter(|c| !c.is_container).count();
        let chapter_count = scanned.len().saturating_sub(top_count);
        crate::events::emit(
            "scan-done",
            serde_json::json!({
                "root_dir": root_dir,
                "comics": top_count,
                "chapters": chapter_count,
            }),
        );
        Ok::<String, String>(format!("重新扫描完成：{top_count} 部漫画，{chapter_count} 个章节"))
    })
    .await
    .map_err(|e| format!("扫描任务失败: {e}"))?
}

/// 重新扫描全部漫画根目录（后台线程执行，完成后 emit scan-done）
#[tauri::command]
pub(crate) async fn rescan_all() -> Result<String, String> {
    let roots = comic_db::get_root_dirs()?;
    if roots.is_empty() {
        return Err("没有已添加的漫画目录".to_string());
    }
    let paths: Vec<String> = roots.into_iter().map(|r| r.path).collect();
    tauri::async_runtime::spawn_blocking(move || {
        let mut total_comics = 0;
        let mut total_chapters = 0;
        let mut errors: Vec<String> = Vec::new();
        for path in &paths {
            let old_covers = comic_db::load_root_cover_paths(path).unwrap_or_default();
            match scanner::scan_directory(path) {
                Ok(comics) => {
                    if let Err(e) = comic_db::replace_root_comics(path, &comics) {
                        errors.push(format!("{path}: {e}"));
                        continue;
                    }
                    for comic in &comics {
                        if !comic.cover_path.is_empty() {
                            if let Some(old) = old_covers.get(&comic.id) {
                                if old != &comic.cover_path {
                                    // cover.* 变化 → 删除本体封面库中该条目，下次请求重新生成
                                    if let Some(bucket) = crate::covers_db::bucket_dir(comic) {
                                        let key = crate::covers_db::rel_key_of(comic, &bucket);
                                        let _ = crate::covers_db::delete_cover(&bucket, &key);
                                    }
                                }
                            }
                        }
                    }
                    // 清理各漫画本体外部封面库中已不存在的章节条目
                    crate::covers_db::purge_after_scan(&comics);
                    let top_count = comics.iter().filter(|c| !c.is_container).count();
                    total_comics += top_count;
                    total_chapters += comics.len().saturating_sub(top_count);
                }
                Err(e) => errors.push(format!("{path}: {e}")),
            }
        }
        crate::events::emit(
            "scan-done",
            serde_json::json!({
                "root_dir": "",
                "comics": total_comics,
                "chapters": total_chapters,
            }),
        );
        if errors.is_empty() {
            Ok::<String, String>(format!("全部目录扫描完成：{total_comics} 部漫画，{total_chapters} 个章节"))
        } else {
            Err(errors.join("\n"))
        }
    })
    .await
    .map_err(|e| format!("扫描任务失败: {e}"))?
}

/// 清空全部漫画封面缓存（下次请求重新生成）
#[tauri::command]
pub(crate) fn clear_cover_cache() -> Result<(), String> {
    // 清空全部本体封面库（唯一来源），下次请求重新生成
    crate::covers_db::clear_all();
    Ok(())
}

// ── 漫画查询与编辑 ──

#[tauri::command]
pub(crate) fn get_top_level_comics() -> Result<Vec<comic_db::Comic>, String> {
    comic_db::load_top_level_comics()
}

#[tauri::command]
pub(crate) fn get_chapters(series_id: String) -> Result<Vec<comic_db::Comic>, String> {
    comic_db::load_chapters(&series_id)
}

#[tauri::command]
pub(crate) fn get_comic(comic_id: String) -> Result<Option<comic_db::Comic>, String> {
    comic_db::get_comic(&comic_id)
}

#[tauri::command]
pub(crate) fn delete_comic(comic_id: String) -> Result<(), String> {
    comic_db::delete_comic(&comic_id)
}

/// 重命名漫画/章节。
/// 章节标题权威为所属系列的 index.json：重命名章节时同步写回，
/// 避免显示时被 index.json 覆盖导致重命名静默失效。
#[tauri::command]
pub(crate) fn rename_comic(comic_id: String, title: String) -> Result<(), String> {
    let comic = comic_db::get_comic(&comic_id)?.ok_or_else(|| "漫画不存在".to_string())?;
    comic_db::update_comic(&comic_id, &title)?;
    // 章节（父为普通系列）→ 同步所属系列目录的 index.json
    if !comic.series_id.is_empty() {
        if let Some(series) = comic_db::get_comic(&comic.series_id)? {
            if !series.is_container {
                let series_dir = Path::new(&series.root_dir).join(&series.path);
                scanner::set_entry_title(&series_dir.to_string_lossy(), &comic.path, &title);
            }
        }
    }
    Ok(())
}

/// 单个系列续读聚合
#[tauri::command]
pub(crate) fn get_series_progress(series_id: String) -> Result<comic_db::SeriesProgress, String> {
    comic_db::get_series_progress(&series_id)
}

/// 全部系列续读聚合（书架一次拉取）
#[tauri::command]
pub(crate) fn get_all_series_progress(
) -> Result<std::collections::HashMap<String, comic_db::SeriesProgress>, String> {
    comic_db::get_all_series_progress()
}

/// 在资源管理器中打开漫画所在目录
#[tauri::command]
pub(crate) fn open_comic_directory(comic_id: String) -> Result<String, String> {
    let comic = comic_db::get_comic(&comic_id)?.ok_or_else(|| "漫画不存在".to_string())?;
    let full = Path::new(&comic.root_dir).join(&comic.path);
    let target = if full.is_dir() {
        full
    } else {
        // 文件类（压缩包）打开父目录
        full.parent().map(|p| p.to_path_buf()).unwrap_or(full)
    };
    if !target.is_dir() {
        return Err("目录不存在".to_string());
    }
    opener::open(&target).map_err(|e| format!("打开目录失败: {e}"))?;
    Ok(target.to_string_lossy().to_string())
}

/// 解析漫画对应的磁盘目录（用于 index.json / completed 操作）
fn resolve_dir(comic_id: &str) -> Result<PathBuf, String> {
    let comic = comic_db::get_comic(comic_id)?.ok_or_else(|| "漫画不存在".to_string())?;
    let full = Path::new(&comic.root_dir).join(&comic.path);
    Ok(if full.is_dir() { full } else { comic.root_dir.into() })
}

/// 同步 index.json 与磁盘状态（重编号 + 保留标题/completed）
#[tauri::command]
pub(crate) fn sync_index(comic_id: String) -> Result<bool, String> {
    let dir = resolve_dir(&comic_id)?;
    if !dir.is_dir() {
        return Err("目录不存在".to_string());
    }
    Ok(scanner::sync_index_json(&dir.to_string_lossy()))
}

/// 读取系列完结标志（DB 缓存优先；未缓存时读 index.json 权威并回填）
#[tauri::command]
pub(crate) fn get_completed_status(comic_id: String) -> Result<Option<bool>, String> {
    if let Some(cached) = comic_db::get_completed(&comic_id)? {
        return Ok(Some(cached));
    }
    let dir = resolve_dir(&comic_id)?;
    let status = scanner::get_completed_status(&dir.to_string_lossy());
    if let Some(s) = status {
        let _ = comic_db::set_completed(&comic_id, s);
    }
    Ok(status)
}

/// 设置系列完结标志（index.json 权威 + DB 缓存同步）
#[tauri::command]
pub(crate) fn set_completed_status(comic_id: String, completed: bool) -> Result<(), String> {
    let dir = resolve_dir(&comic_id)?;
    scanner::set_completed_status(&dir.to_string_lossy(), completed)?;
    let _ = comic_db::set_completed(&comic_id, completed);
    Ok(())
}

/// 一次读取全部顶层漫画的完结标志（书架渲染用）。
/// DB 缓存命中直接返回；未缓存条目回退读 index.json 并回填（无标记也回填 false），
/// 保证磁盘只扫一次，后续书架加载纯 DB 读，不随目录数变慢。
#[tauri::command]
pub(crate) fn get_all_completed_status() -> Result<std::collections::HashMap<String, bool>, String> {
    let comics = comic_db::load_top_level_comics()?;
    let mut map = std::collections::HashMap::new();
    for comic in &comics {
        if let Some(cached) = comic_db::get_completed(&comic.id)? {
            map.insert(comic.id.clone(), cached);
            continue;
        }
        let dir = Path::new(&comic.root_dir).join(&comic.path);
        let completed = if dir.is_dir() {
            scanner::get_completed_status(&dir.to_string_lossy())
        } else {
            None
        };
        let value = completed.unwrap_or(false);
        let _ = comic_db::set_completed(&comic.id, value);
        map.insert(comic.id.clone(), value);
    }
    Ok(map)
}

// ── 封面 & 页面 ──

/// 漫画封面字节（HTTP 流式接口与 data URL 命令共用）
pub(crate) fn comic_cover_bytes(comic_id: &str) -> Result<(String, Vec<u8>), String> {
    let comic = comic_db::get_comic(comic_id)?.ok_or_else(|| "漫画不存在".to_string())?;
    let cover = covers::get_cover(&comic)?;
    Ok((cover.mime, cover.data))
}

#[tauri::command]
pub(crate) async fn get_comic_cover_data_url(comic_id: String) -> Result<String, String> {
    // 封面读取/生成（解码 + 缩放 + jpeg 编码）是 CPU 密集操作，放后台线程避免阻塞 UI 主线程
    tauri::async_runtime::spawn_blocking(move || {
        let (mime, bytes) = comic_cover_bytes(&comic_id)?;
        Ok(data_url(&mime, &bytes))
    })
    .await
    .map_err(|e| format!("封面生成失败: {e}"))?
}

/// 页面列表（一次调用获取全部页名，阅读器按索引请求页面数据）
#[tauri::command]
pub(crate) fn list_pages(comic_id: String) -> Result<Vec<PageInfo>, String> {
    let comic = comic_db::get_comic(&comic_id)?.ok_or_else(|| "漫画不存在".to_string())?;
    if !matches!(comic.kind.as_str(), "folder" | "archive") {
        return Err("该条目无独立页面".to_string());
    }
    let files = scanner::list_page_files(&comic);
    Ok(files
        .iter()
        .enumerate()
        .map(|(i, n)| PageInfo {
            index: i,
            name: n.clone(),
            mime: scanner::mime_for(n),
        })
        .collect())
}

/// 漫画页面字节（HTTP 流式接口与 data URL 命令共用）
pub(crate) fn comic_page_bytes(comic_id: &str, page_index: usize) -> Result<(String, Vec<u8>), String> {
    let comic = comic_db::get_comic(comic_id)?.ok_or_else(|| "漫画不存在".to_string())?;
    let (bytes, mime) = scanner::read_page(&comic, page_index).ok_or_else(|| "页面读取失败".to_string())?;
    Ok((mime, bytes))
}

#[tauri::command]
pub(crate) async fn get_page_data_url(comic_id: String, page_index: usize) -> Result<String, String> {
    // 页面文件读取放后台线程，避免阻塞 UI 主线程
    tauri::async_runtime::spawn_blocking(move || {
        let (mime, bytes) = comic_page_bytes(&comic_id, page_index)?;
        Ok(data_url(&mime, &bytes))
    })
    .await
    .map_err(|e| format!("页面读取失败: {e}"))?
}

/// 页索引：外部封面库缓存优先（校验预览宽度），无缓存则构建并保存
fn page_index_cached(comic: &comic_db::Comic) -> Result<covers::PageIndex, String> {
    if let Some(bucket) = crate::covers_db::bucket_dir(comic) {
        let key = crate::covers_db::rel_key_of(comic, &bucket);
        if let Some(cached) = crate::covers_db::load_page_index(&bucket, &key)? {
            if let Ok(idx) = serde_json::from_str::<covers::PageIndex>(&cached) {
                if idx.pw == covers::PREVIEW_WIDTH {
                    return Ok(idx);
                }
            }
        }
        let idx = covers::build_page_index(comic)?;
        let data = serde_json::to_string(&idx).map_err(|e| format!("序列化失败: {e}"))?;
        let _ = crate::covers_db::save_page_index(&bucket, &key, &data);
        return Ok(idx);
    }
    // 无本体目录（不应发生）→ 直接构建，不缓存
    covers::build_page_index(comic)
}

/// 章节页面条带索引（自定义封面裁剪预览用，外部封面库缓存 + 校验预览宽度）
#[tauri::command]
pub(crate) async fn get_comic_page_index(comic_id: String) -> Result<covers::PageIndex, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let comic = comic_db::get_comic(&comic_id)?.ok_or_else(|| "漫画不存在".to_string())?;
        page_index_cached(&comic)
    })
    .await
    .map_err(|e| format!("分析章节失败: {e}"))?
}

/// 漫画页面预览字节（HTTP 流式接口与 data URL 命令共用；外部封面库缓存命中直接返回）
pub(crate) fn comic_preview_bytes(
    comic_id: &str,
    page_index: usize,
) -> Result<(String, Vec<u8>), String> {
    let comic = comic_db::get_comic(comic_id)?.ok_or_else(|| "漫画不存在".to_string())?;
    let bucket = crate::covers_db::bucket_dir(&comic)
        .ok_or_else(|| "无法定位漫画本体".to_string())?;
    let key = crate::covers_db::rel_key_of(&comic, &bucket);
    if let Some((data, mime)) =
        crate::covers_db::load_preview(&bucket, &key, page_index)?
    {
        return Ok((mime, data));
    }
    let (bytes, _mime) =
        scanner::read_page(&comic, page_index).ok_or_else(|| "页面读取失败".to_string())?;
    match covers::resize_to_width(&bytes, covers::PREVIEW_WIDTH) {
        Some((data, mime, w, h)) => {
            let _ = crate::covers_db::save_preview(
                &bucket,
                &key,
                page_index,
                &data,
                &mime,
                w as i64,
                h as i64,
            );
            Ok((mime, data))
        }
        None => Err("预览生成失败".to_string()),
    }
}

/// 页面预览缩略图（按 400 宽缩放，拼接条带用）。外部封面库缓存命中直接返回，未命中生成并缓存。
#[tauri::command]
pub(crate) async fn get_page_preview_data_url(
    comic_id: String,
    page_index: usize,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (mime, bytes) = comic_preview_bytes(&comic_id, page_index)?;
        Ok(data_url(&mime, &bytes))
    })
    .await
    .map_err(|e| format!("预览生成失败: {e}"))?
}

/// 自定义章节封面：从章节拼接条带中按像素偏移截取 5:7 窗口生成封面。
/// 偏移同时存入外部封面库 configs（弹窗再次打开时恢复位置，随目录走）。
#[tauri::command]
pub(crate) async fn set_comic_cover_from_offset(
    comic_id: String,
    offset: i64,
) -> Result<(), String> {
    // 读页 + 拼接裁剪放后台线程，避免阻塞 UI 主线程
    tauri::async_runtime::spawn_blocking(move || {
        let comic = comic_db::get_comic(&comic_id)?.ok_or_else(|| "漫画不存在".to_string())?;
        let idx = page_index_cached(&comic)?;
        let target = covers::COVER_SIZE;
        let offset = offset.max(0) as u32;
        let (data, mime, w, h) = covers::generate_cropped_cover(&comic, &idx, offset, target)
            .map_err(|e| format!("封面生成失败: {e}"))?;
        // 写入本体外部封面库（唯一来源）+ 裁剪偏移参数（随目录走）
        if let Some(bucket) = crate::covers_db::bucket_dir(&comic) {
            let key = crate::covers_db::rel_key_of(&comic, &bucket);
            let _ = crate::covers_db::save_cover(
                &bucket,
                &key,
                &data,
                &mime,
                w as i64,
                h as i64,
            );
            let mut config = std::collections::HashMap::new();
            config.insert("cover_crop_offset".to_string(), offset.to_string());
            let _ = crate::covers_db::save_configs(&bucket, &key, &config);
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("封面生成失败: {e}"))?
}

/// 恢复章节封面为自动生成（清除本体封面库中的封面，下次请求重新生成）
#[tauri::command]
pub(crate) fn reset_comic_cover(comic_id: String) -> Result<(), String> {
    let comic = comic_db::get_comic(&comic_id)?.ok_or_else(|| "漫画不存在".to_string())?;
    if let Some(bucket) = crate::covers_db::bucket_dir(&comic) {
        let key = crate::covers_db::rel_key_of(&comic, &bucket);
        let _ = crate::covers_db::delete_cover(&bucket, &key);
    }
    Ok(())
}

/// 读取 PDF 章节原始字节（前端 pdf.js 渲染），base64 返回
#[tauri::command]
pub(crate) async fn get_comic_pdf_data(comic_id: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let comic = comic_db::get_comic(&comic_id)?.ok_or_else(|| "漫画不存在".to_string())?;
        if comic.kind != "pdf" {
            return Err("该条目不是 PDF".to_string());
        }
        let full = Path::new(&comic.root_dir).join(&comic.path);
        let bytes = std::fs::read(&full).map_err(|e| format!("读取 PDF 失败: {e}"))?;
        Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
    })
    .await
    .map_err(|e| format!("读取 PDF 失败: {e}"))?
}

/// 上传封面 data URL 保存为 standard 封面。
/// overwrite=false（默认）：已有封面则跳过（PDF 首次自动上传）；
/// overwrite=true：覆盖已有封面（PDF 手动截取选封面）。
#[tauri::command]
pub(crate) fn upload_comic_cover(
    comic_id: String,
    data_url_value: String,
    overwrite: Option<bool>,
) -> Result<String, String> {
    let comic = comic_db::get_comic(&comic_id)?.ok_or_else(|| "漫画不存在".to_string())?;
    let exists = crate::covers_db::bucket_dir(&comic)
        .map(|bucket| {
            let key = crate::covers_db::rel_key_of(&comic, &bucket);
            crate::covers_db::load_cover(&bucket, &key)
                .ok()
                .flatten()
                .is_some()
        })
        .unwrap_or(false);
    if !overwrite.unwrap_or(false) && exists {
        return Ok("已存在".to_string());
    }
    let (mime, bytes) = decode_data_url(&data_url_value).ok_or_else(|| "封面数据无效".to_string())?;
    let (w, h) = image::load_from_memory(&bytes)
        .map(|img| img.dimensions())
        .unwrap_or((0, 0));
    // 写入本体外部封面库（唯一来源）
    if let Some(bucket) = crate::covers_db::bucket_dir(&comic) {
        let key = crate::covers_db::rel_key_of(&comic, &bucket);
        let _ = crate::covers_db::save_cover(&bucket, &key, &bytes, &mime, w as i64, h as i64);
    }
    Ok("ok".to_string())
}

// ── 阅读进度 & 配置 ──

#[tauri::command]
pub(crate) fn set_reading_progress(comic_id: String, page: i64) -> Result<(), String> {
    comic_db::set_reading_progress(&comic_id, page)
}

#[tauri::command]
pub(crate) fn get_reading_progress(comic_id: String) -> Result<i64, String> {
    comic_db::get_reading_progress(&comic_id)
}

/// 读取单漫画参数（外部封面库 configs：手动封面裁剪偏移等，随目录走）
#[tauri::command]
pub(crate) fn get_comic_config(comic_id: String) -> Result<std::collections::HashMap<String, String>, String> {
    let comic = comic_db::get_comic(&comic_id)?.ok_or_else(|| "漫画不存在".to_string())?;
    if let Some(bucket) = crate::covers_db::bucket_dir(&comic) {
        let key = crate::covers_db::rel_key_of(&comic, &bucket);
        crate::covers_db::load_configs(&bucket, &key)
    } else {
        Ok(std::collections::HashMap::new())
    }
}

/// 保存单漫画参数（外部封面库 configs，随目录走）
#[tauri::command]
pub(crate) fn set_comic_config(
    comic_id: String,
    config: std::collections::HashMap<String, String>,
) -> Result<(), String> {
    let comic = comic_db::get_comic(&comic_id)?.ok_or_else(|| "漫画不存在".to_string())?;
    if let Some(bucket) = crate::covers_db::bucket_dir(&comic) {
        let key = crate::covers_db::rel_key_of(&comic, &bucket);
        crate::covers_db::save_configs(&bucket, &key, &config)
    } else {
        Ok(())
    }
}

#[tauri::command]
pub(crate) fn batch_set_sort_order(orders: Vec<(i64, String)>) -> Result<(), String> {
    comic_db::batch_set_sort_order(&orders)
}

