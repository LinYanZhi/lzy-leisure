//! 漫画目录扫描器——文件工具/常量层。
//! 原 scanner.rs 拆分（index.json 处理见 scanner_index，扫描编排见 scanner.rs）：
//! 扩展名判断、目录分类（DirClass）、PDF/压缩包页数缓存、封面查找。
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 元数据文件名（不作为页面）
const META_FILES: [&str; 2] = ["cover", "index"];

const IMAGE_EXTENSIONS: [&str; 9] = [
    ".jpg", ".jpeg", ".png", ".webp", ".gif", ".bmp", ".tiff", ".tif", ".avif",
];
const ARCHIVE_EXTENSIONS: [&str; 3] = [".zip", ".cbz", ".rar"];
const CBR_EXTENSIONS: [&str; 2] = [".cbr", ".cb7"];

pub(crate) fn is_image_file(name: &str) -> bool {
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e.to_lowercase()))
        .unwrap_or_default();
    IMAGE_EXTENSIONS.contains(&ext.as_str())
}

/// 是否为元数据文件（cover.* / index.*）
pub(crate) fn is_meta_file(name: &str) -> bool {
    let stem = Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_lowercase();
    META_FILES.contains(&stem.as_str())
}

pub(crate) fn is_archive(name: &str) -> bool {
    let lower = name.to_lowercase();
    ARCHIVE_EXTENSIONS
        .iter()
        .chain(CBR_EXTENSIONS.iter())
        .any(|ext| lower.ends_with(ext))
}

/// PDF 文件（前端 pdf.js 渲染阅读）
pub(crate) fn is_pdf(name: &str) -> bool {
    name.to_lowercase().ends_with(".pdf")
}

/// zip/cbz 可读取的压缩包
pub(crate) fn is_readable_archive(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".zip") || lower.ends_with(".cbz")
}

pub(crate) fn is_hidden_dir(name: &str) -> bool {
    name.starts_with('.') || name.starts_with('_')
}

/// 单次遍历目录的分类结果。
/// 扫描只需要图片数量/大小与章节条目，不依赖图片顺序 → 不做 natural_key 排序，
/// 也避免对同一目录多次 read_dir（find_cover / 章节收集 / 图片计数各一趟的开销）。
pub(crate) struct DirClass {
    /// 可见子目录名（排除隐藏/元数据文件）
    pub(crate) dirs: Vec<String>,
    /// 压缩包文件名（zip/cbz/rar 等）
    pub(crate) archives: Vec<String>,
    /// PDF 文件路径
    pub(crate) pdfs: Vec<PathBuf>,
    /// 图片文件：(路径, 大小)——大小取自目录枚举缓存，
    /// Windows 上 DirEntry::metadata 复用 FindNextFile 的 WIN32_FIND_DATA，无额外系统调用。
    /// USB/外接盘上每文件一次 stat 都很昂贵，必须避免事后 Path::metadata()。
    pub(crate) images: Vec<(PathBuf, u64)>,
    /// cover.* 封面文件名
    pub(crate) cover: Option<String>,
}

/// 单次 read_dir 分类目录内容（扫描主路径专用）
pub(crate) fn classify_dir(dir: &Path) -> DirClass {
    let mut out = DirClass {
        dirs: Vec::new(),
        archives: Vec::new(),
        pdfs: Vec::new(),
        images: Vec::new(),
        cover: None,
    };
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if is_hidden_dir(&name) {
            continue;
        }
        if is_meta_file(&name) {
            let stem = Path::new(&name)
                .file_stem()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if stem == "cover" && is_image_file(&name) {
                out.cover = Some(name);
            }
            continue;
        }
        // 用 DirEntry::file_type()：Windows 上直接来自目录枚举缓存，无需额外系统调用
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_dir() {
            out.dirs.push(name);
        } else if is_pdf(&name) {
            out.pdfs.push(entry.path());
        } else if is_archive(&name) {
            out.archives.push(name);
        } else if is_image_file(&name) {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            out.images.push((entry.path(), size));
        }
    }
    out
}

/// PDF 页数（lopdf 解析文档结构，不渲染）
pub(crate) fn pdf_page_count(path: &Path) -> i64 {
    let Ok(bytes) = std::fs::read(path) else {
        return 0;
    };
    lopdf::Document::load_mem(&bytes)
        .map(|doc| doc.get_pages().len() as i64)
        .unwrap_or(0)
}

/// 页数增量缓存：rel 路径 + 文件大小 → 页数。
/// 重扫时若文件未变化则跳过 PDF/zip 解析，大幅提速。
pub(crate) type PageCache = HashMap<(String, i64), i64>;

/// 命中缓存直接返回页数，否则计算（代价：PDF 全文解析 / zip 中央目录读取）
pub(crate) fn cached_pages(
    cache: &Option<PageCache>,
    rel: &str,
    size: i64,
    compute: impl FnOnce() -> i64,
) -> i64 {
    if let Some(map) = cache {
        if let Some(&n) = map.get(&(rel.to_string(), size)) {
            return n;
        }
    }
    compute()
}

/// 查找目录下的 cover.* 文件
pub(crate) fn find_cover_file(dir: &Path) -> Option<String> {
    if !dir.is_dir() {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if is_meta_file(&name) && is_image_file(&name) {
            let stem = Path::new(&name)
                .file_stem()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if stem == "cover" {
                return Some(name);
            }
        }
    }
    None
}
