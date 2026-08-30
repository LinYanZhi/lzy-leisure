//! 故事会命令。原 commands.rs 拆分，外部路径 `commands::xxx` 不变。
use super::{data_url, decode_data_url};
use base64::Engine as _;
use std::path::{Path, PathBuf};

// ══════════════════════════════════════════════════════════
//  故事会（单一路径：根目录下每个子目录 = 一年，内含 PDF 期数）
// ══════════════════════════════════════════════════════════

use crate::db::storyclub as story_db;

/// 期数归属的年份数（去重）
fn story_years(issues: &[story_db::StoryIssue]) -> usize {
    issues
        .iter()
        .filter(|i| !i.year.is_empty())
        .map(|i| i.year.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len()
}

/// 设置故事会根路径（单一路径，重新设置即替换旧路径）。
/// 扫描只做纯目录/文件枚举（页数零解析，秒级完成），入库并 emit storyclub-scan-done；
/// 期数页数在打开阅读器时由 pdf.js 解析后回写（storyclub_update_page_count）。
#[tauri::command]
pub(crate) async fn storyclub_set_root(path: String) -> Result<String, String> {
    if !Path::new(&path).is_dir() {
        return Err(format!("目录不存在: {path}"));
    }
    let issues = {
        let path2 = path.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let issues = crate::story_scanner::scan_root_meta(&path2)?;
            story_db::replace_root_issues(&issues)?;
            Ok::<_, String>(issues)
        })
        .await
        .map_err(|e| format!("扫描任务失败: {e}"))??
    };
    story_db::set_root(&path)?;
    // 封面库随根目录走：清理新根封面库中已不存在的期数（换根后旧库文件留在原目录，由用户自行处理）
    let ids: Vec<String> = issues.iter().map(|i| i.id.clone()).collect();
    let _ = crate::story_covers::prune_orphans(&ids);
    // 后台静默解析缺页数的期数（增量；解析完 emit storyclub-pages-done，前端刷新显示）
    crate::story_pages::start();
    // 后台迁移分库前的旧单一封面库到各年库并压缩原图（防并发；无旧库时秒退）
    crate::story_covers::start_migrate_and_compress();
    let years = story_years(&issues);
    crate::events::emit(
        "storyclub-scan-done",
        serde_json::json!({ "root_dir": path, "issues": issues.len(), "years": years }),
    );
    Ok(format!("扫描完成：{years} 年，共 {} 期", issues.len()))
}

/// 读取当前故事会根路径（未设置返回 null）
#[tauri::command]
pub(crate) fn storyclub_get_root() -> Result<Option<String>, String> {
    story_db::get_root()
}

/// 移除故事会根路径（记录 + 期数清空；不删本地文件）
#[tauri::command]
pub(crate) fn storyclub_remove_root() -> Result<(), String> {
    story_db::remove_root()
}

/// 重新扫描当前根路径（纯目录/文件枚举，页数零解析）
#[tauri::command]
pub(crate) async fn storyclub_rescan() -> Result<String, String> {
    let root = story_db::get_root()?.ok_or_else(|| "尚未设置故事会目录".to_string())?;
    let path = root.clone();
    let issues = tauri::async_runtime::spawn_blocking(move || {
        let issues = crate::story_scanner::scan_root_meta(&root)?;
        story_db::replace_root_issues(&issues)?;
        Ok::<_, String>(issues)
    })
    .await
    .map_err(|e| format!("扫描任务失败: {e}"))??;
    // 清理封面库中已不存在的期数（文件被删除/移动后避免孤儿封面残留）
    let ids: Vec<String> = issues.iter().map(|i| i.id.clone()).collect();
    let _ = crate::story_covers::prune_orphans(&ids);
    // 后台静默解析缺页数的期数（增量；解析完 emit storyclub-pages-done，前端刷新显示）
    crate::story_pages::start();
    // 后台迁移分库前的旧单一封面库到各年库并压缩原图（防并发；无旧库时秒退）
    crate::story_covers::start_migrate_and_compress();
    let years = story_years(&issues);
    crate::events::emit(
        "storyclub-scan-done",
        serde_json::json!({ "root_dir": path, "issues": issues.len(), "years": years }),
    );
    Ok(format!("重新扫描完成：{years} 年，共 {} 期", issues.len()))
}

/// 全部期数（前端按年份分组展示）
#[tauri::command]
pub(crate) fn storyclub_list_issues() -> Result<Vec<story_db::StoryIssue>, String> {
    // 进入书架即尝试启动后台页数解析（防并发；无缺页数期数时秒退）——
    // 覆盖「升级后存量 0 值期数」不重扫也能被后台补齐页数
    crate::story_pages::start();
    // 同样兜底启动封面库迁移压缩（升级后首次进入书架即开始收敛旧大库）
    crate::story_covers::start_migrate_and_compress();
    story_db::list_issues()
}

/// 前端打开阅读器后回写真实页数（pdf.js 解析；与后台解析幂等）
#[tauri::command]
pub(crate) fn storyclub_update_page_count(issue_id: String, page_count: i64) -> Result<(), String> {
    if page_count > 0 {
        story_db::set_page_count(&issue_id, page_count)?;
    }
    Ok(())
}

/// 读取期数 PDF 原始字节（前端 pdf.js 渲染），base64 返回
#[tauri::command]
pub(crate) async fn storyclub_get_pdf_data(issue_id: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let issue = story_db::get_issue(&issue_id)?.ok_or_else(|| "期数不存在".to_string())?;
        let bytes = std::fs::read(&issue.path).map_err(|e| format!("读取 PDF 失败: {e}"))?;
        Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
    })
    .await
    .map_err(|e| format!("读取 PDF 失败: {e}"))?
}

/// 读取期数 PDF 磁盘路径（桌面端 asset 协议流式加载用）
#[tauri::command]
pub(crate) fn storyclub_get_pdf_path(issue_id: String) -> Result<String, String> {
    let issue = story_db::get_issue(&issue_id)?.ok_or_else(|| "期数不存在".to_string())?;
    Ok(issue.path)
}

/// 保存期数阅读进度（page = 当前跨页的第一页页码，0 起）
#[tauri::command]
pub(crate) fn storyclub_set_progress(issue_id: String, page: i64) -> Result<(), String> {
    story_db::set_reading_progress(&issue_id, page)
}

/// 打开期数所在目录（只读）。Windows 上用资源管理器打开并默认选中该 PDF 文件，
/// 用户一眼就能认出是哪一个；非 Windows 或 explorer 不可用时回退打开父目录。
#[tauri::command]
pub(crate) fn storyclub_open_folder(issue_id: String) -> Result<String, String> {
    let issue = story_db::get_issue(&issue_id)?.ok_or_else(|| "期数不存在".to_string())?;
    let p = Path::new(&issue.path);
    if !p.exists() {
        return Err(format!("文件不存在: {}", p.display()));
    }
    let dir = p.parent().unwrap_or(p);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // explorer /select 定位并选中文件（路径带空格时需整体加引号）。
        // 必须用 raw_arg 原样传参：Command::arg 会把含引号的参数再转义一层
        // （外层加引号、内部引号变 \"），explorer 解析转义引号会失败而停在默认位置
        let arg = format!("/select,\"{}\"", p.to_string_lossy());
        if std::process::Command::new("explorer").raw_arg(arg).spawn().is_ok() {
            return Ok(p.to_string_lossy().to_string());
        }
    }
    if !dir.exists() {
        return Err(format!("目录不存在: {}", dir.display()));
    }
    opener::open(dir).map_err(|e| format!("打开目录失败: {e}"))?;
    Ok(dir.to_string_lossy().to_string())
}

/// 打开年份所在目录（资源管理器定位并选中该年份目录本身，不进入浏览）。
/// 与期数卡（选中 PDF 文件）不同：年份卡对应根目录下的一个子目录。
#[tauri::command]
pub(crate) fn storyclub_open_year_dir(year: String) -> Result<String, String> {
    let root = story_db::get_root()?.ok_or_else(|| "尚未设置故事会目录".to_string())?;
    // 未分类期数直接放根目录；其余年份为根目录下的子目录
    let dir = if year.is_empty() || year == "未分类" {
        PathBuf::from(&root)
    } else {
        Path::new(&root).join(&year)
    };
    if !dir.is_dir() {
        return Err(format!("目录不存在: {}", dir.display()));
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // explorer /select 选中目录本身（父目录中高亮该文件夹），与期数一致用 raw_arg 原样传参
        let arg = format!("/select,\"{}\"", dir.to_string_lossy());
        if std::process::Command::new("explorer").raw_arg(arg).spawn().is_ok() {
            return Ok(dir.to_string_lossy().to_string());
        }
    }
    opener::open(&dir).map_err(|e| format!("打开目录失败: {e}"))?;
    Ok(dir.to_string_lossy().to_string())
}

/// 故事会封面字节（HTTP 流式接口与 data URL 命令共用）
pub(crate) fn storyclub_cover_bytes(issue_id: &str) -> Result<(String, Vec<u8>), String> {
    let bytes = crate::story_covers::get_cover(issue_id)?
        .ok_or_else(|| "该期数尚未生成封面".to_string())?;
    Ok(("image/jpeg".to_string(), bytes))
}

/// 获取期数封面 data URL（封面存故事会根目录下独立封面库；旧散落文件惰性迁移入库）
#[tauri::command]
pub(crate) fn storyclub_get_cover_data_url(issue_id: String) -> Result<String, String> {
    let (mime, bytes) = storyclub_cover_bytes(&issue_id)?;
    Ok(data_url(&mime, &bytes))
}

/// 批量取封面 data URL（书架刷新一次拉全当前视图封面，避免逐张 IPC 卡顿）。
/// 异步执行：批量读取可能触发惰性压缩（解码+重编码），放线程池避免阻塞 UI。
#[tauri::command]
pub(crate) async fn storyclub_get_covers_batch(
    issue_ids: Vec<String>,
) -> Result<Vec<(String, String)>, String> {
    tauri::async_runtime::spawn_blocking(move || crate::story_covers::get_covers_batch(&issue_ids))
        .await
        .map_err(|e| format!("批量封面线程异常: {e}"))?
}

/// 上传期数封面 data URL（前端 pdf.js 渲染第一页生成；写入故事会根目录封面库）
#[tauri::command]
pub(crate) fn storyclub_upload_cover(issue_id: String, data_url_value: String) -> Result<(), String> {
    story_db::get_issue(&issue_id)?.ok_or_else(|| "期数不存在".to_string())?;
    let (_mime, bytes) = decode_data_url(&data_url_value).ok_or_else(|| "封面数据无效".to_string())?;
    crate::story_covers::upsert(&issue_id, &bytes)
}

/// 故事会第一页 JPEG 字节（HTTP 流式接口与 data URL 命令共用）
pub(crate) fn storyclub_first_page_bytes(issue_id: &str) -> Result<(String, Vec<u8>), String> {
    let issue = story_db::get_issue(issue_id)?.ok_or_else(|| "期数不存在".to_string())?;
    let bytes = crate::story_covers::first_page_jpeg_bytes(Path::new(&issue.path))?;
    Ok(("image/jpeg".to_string(), bytes))
}

/// 故事会第 idx 页（0 起）JPEG 字节（HTTP 图片流路由共用；手机端逐页阅读）
pub(crate) fn storyclub_page_bytes(issue_id: &str, idx: usize) -> Result<(String, Vec<u8>), String> {
    let issue = story_db::get_issue(issue_id)?.ok_or_else(|| "期数不存在".to_string())?;
    let bytes = crate::story_covers::page_jpeg_bytes(Path::new(&issue.path), idx)?;
    Ok(("image/jpeg".to_string(), bytes))
}

/// 后端直接提取第一页 JPEG（扫描件封面页多为整页 DCTDecode 图片，秒级取出，免去 pdf.js 解析整个大 PDF），
/// 并按目标宽度降采样重编码（整页扫描图动辄 1-5MB，封面显示仅需数百 px；解码/缩放在线程池执行不阻塞 UI）。
/// 第一页无整页 JPEG 图片或解码异常时返回 Err，由前端退回 pdf.js 渲染。
#[tauri::command]
pub(crate) async fn storyclub_first_page_jpeg(issue_id: String) -> Result<String, String> {
    let (mime, bytes) = tauri::async_runtime::spawn_blocking(move || {
        storyclub_first_page_bytes(&issue_id)
    })
    .await
    .map_err(|e| format!("封面提取线程异常: {e}"))??;
    Ok(data_url(&mime, &bytes))
}

/// 尚未生成封面的期数 id 列表（书架静默补齐用；封面存故事会根目录下封面库）。
/// 已过滤损坏/不可读的 PDF——封面无法生成，前端无需为其等待超时。
#[tauri::command]
pub(crate) fn storyclub_missing_covers() -> Result<Vec<String>, String> {
    let mut missing = crate::story_covers::missing_cover_ids()?;
    missing.retain(|id| {
        story_db::get_issue(id)
            .ok()
            .flatten()
            .map(|i| crate::story_covers::pdf_quick_check(Path::new(&i.path)))
            .unwrap_or(false)
    });
    Ok(missing)
}
