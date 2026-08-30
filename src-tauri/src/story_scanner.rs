//! 故事会扫描器
//!
//! 目标目录结构（用户数据）：
//!   D:\文本\故事会\
//!     ├── 故事会-2019\          ← 每个子目录 = 一年
//!     │   ├── 2019第01期.pdf
//!     │   └── ...
//!     └── ...
//!
//! 规则：
//!   - 根目录下的每个子目录视为一年（跳过隐藏/以 _ 开头目录）
//!   - 年份目录下的 *.pdf 文件视为期数
//!   - 兼容：根目录直接放 PDF 时归入空年份
//!
//! 扫描只做纯目录/文件枚举（秒级）：期数页数不做任何解析，
//! 打开阅读器时由前端 pdf.js 解析后回写（storyclub_update_page_count）。
use crate::db::storyclub::StoryIssue;
use std::path::{Path, PathBuf};

fn is_hidden_dir(name: &str) -> bool {
    name.starts_with('.') || name.starts_with('_')
}

fn is_pdf(name: &str) -> bool {
    name.to_lowercase().ends_with(".pdf")
}

/// 收集目录下的 PDF 文件名（自然排序）
fn collect_pdfs(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if is_pdf(&name) {
                names.push(name);
            }
        }
    }
    names.sort_by_key(|n| crate::db::natural_key(n));
    names
}

/// 内容指纹：文件大小 + 前 64KB 内容哈希（md5）。
/// 文件改名/移动后内容不变、指纹不变，重扫时据此归并原记录保留进度。
/// 只读文件头 64KB，秒级完成，不做 PDF 解析；读取失败返回空串（降级为按路径匹配）。
fn file_fingerprint(path: &Path, size: i64) -> String {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return String::new();
    };
    let mut buf = [0u8; 65536];
    let n = file.read(&mut buf).unwrap_or(0);
    format!("{size}:{:x}", md5::compute(&buf[..n]))
}

/// 枚举根目录下全部期数（仅目录/文件枚举 + 头部 64KB 读取，不解析 PDF；页数恒为 0）。
/// 扫描即此一步：秒级完成，结果立即整表入库。
pub fn scan_root_meta(root: &str) -> Result<Vec<StoryIssue>, String> {
    let root_path = Path::new(root);
    if !root_path.is_dir() {
        return Err(format!("目录不存在: {root}"));
    }

    let mut issues: Vec<StoryIssue> = Vec::new();

    // 1. 年份目录
    let mut year_dirs: Vec<String> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(root_path) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if is_hidden_dir(&name) {
                continue;
            }
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_dir() {
                year_dirs.push(name);
            }
        }
    }
    year_dirs.sort_by_key(|n| crate::db::natural_key(n));
    for year in &year_dirs {
        let dir = root_path.join(year);
        let pdfs = collect_pdfs(&dir);
        for pdf in &pdfs {
            let full: PathBuf = dir.join(pdf);
            let size = full.metadata().map(|m| m.len() as i64).unwrap_or(0);
            let title = Path::new(pdf)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| pdf.clone());
            let full_str = full.to_string_lossy().to_string();
            issues.push(StoryIssue {
                id: crate::db::video_id(&full_str),
                title,
                year: year.clone(),
                path: full_str,
                page_count: 0,
                size,
                reading_progress: 0,
                sort_order: 0,
                fingerprint: file_fingerprint(&full, size),
                updated_at: String::new(),
            });
        }
    }

    // 2. 根目录直放 PDF（无年份归属）
    {
        let pdfs = collect_pdfs(root_path);
        for pdf in &pdfs {
            let full = root_path.join(pdf);
            let size = full.metadata().map(|m| m.len() as i64).unwrap_or(0);
            let title = Path::new(pdf)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| pdf.clone());
            let full_str = full.to_string_lossy().to_string();
            issues.push(StoryIssue {
                id: crate::db::video_id(&full_str),
                title,
                year: String::new(),
                path: full_str,
                page_count: 0,
                size,
                reading_progress: 0,
                sort_order: 0,
                fingerprint: file_fingerprint(&full, size),
                updated_at: String::new(),
            });
        }
    }

    Ok(issues)
}
