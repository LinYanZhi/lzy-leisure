//! 漫画目录扫描器
//!
//! 支持结构：
//!   - 根目录直接含图片 → 单本 folder 漫画
//!   - 根目录含子目录/压缩包 → series + chapters 层级
//!   - cover.* 文件作为封面（不列入页面）
//!   - index.json 提供章节标题覆盖与排序
//!   - zip/cbz 统计页数；rar/cbr 仅登记（读取后续支持）
//!
//! 浏览器保存物清理（.下载 / _files / .css）为高级功能，暂不在此实现。
//!
//! 按职责拆分为子模块（外部 crate::scanner::* 路径不变）：
//!   - `scanner_fs.rs`   文件工具/常量：扩展名判断、目录分类、页数缓存、封面查找
//!   - `scanner_index.rs` index.json 处理：角色推断、条目收集/同步、标题与完结状态
//!   - 本文件保留扫描编排（单元识别/集合/系列/单本）与页面读取
pub(crate) mod scanner_fs;
pub(crate) mod scanner_index;

use scanner_fs::{
    cached_pages, classify_dir, find_cover_file, is_archive, is_hidden_dir, is_image_file,
    is_meta_file, is_pdf, is_readable_archive, pdf_page_count, PageCache,
};
use scanner_index::{collect_chapter_entries, infer_role, write_role_if_missing};

// 重导出：保持 crate::scanner::* 外部路径不变
pub use scanner_index::{
    ensure_index_json, get_completed_status, load_index_json, load_index_meta, set_completed_status,
    set_entry_title, sync_index_json, ROLE_COLLECTION, ROLE_COMIC, ROLE_SINGLE,
};

use crate::db::{self, comics::Comic};
use std::path::Path;

// ══════════════════════════════════════════════════════════
//  单条目扫描
// ══════════════════════════════════════════════════════════

/// 压缩包页数（zip/cbz 统计；rar/cbr 返回 0）
fn archive_page_count(path: &Path, name: &str) -> i64 {
    if !is_readable_archive(name) {
        return 0;
    }
    let Ok(file) = std::fs::File::open(path) else {
        return 0;
    };
    let Ok(archive) = zip::ZipArchive::new(file) else {
        return 0;
    };
    let count = archive
        .file_names()
        .filter(|n| !n.ends_with('/') && is_image_file(n))
        .count();
    count as i64
}

fn scan_archive(filepath: &Path, root: &str, rel_path: &str, cache: &Option<PageCache>) -> Option<Comic> {
    let name = Path::new(rel_path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = filepath
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let title = Path::new(rel_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let size = filepath.metadata().map(|m| m.len() as i64).unwrap_or(0);
    let pages = cached_pages(cache, rel_path, size, || archive_page_count(filepath, &name));
    Some(Comic::new(
        db::generate_id(root, rel_path),
        title,
        rel_path.to_string(),
        "archive".to_string(),
        format!(".{ext}"),
        pages,
        size,
        root.to_string(),
        String::new(),
        String::new(),
        0,
    ))
}

/// 扫描 PDF 文件（kind="pdf"，页数由 lopdf 解析）
fn scan_pdf(filepath: &Path, root: &str, rel_path: &str, cache: &Option<PageCache>) -> Option<Comic> {
    let title = Path::new(rel_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let size = filepath.metadata().map(|m| m.len() as i64).unwrap_or(0);
    let pages = cached_pages(cache, rel_path, size, || pdf_page_count(filepath));
    Some(Comic::new(
        db::generate_id(root, rel_path),
        title,
        rel_path.to_string(),
        "pdf".to_string(),
        ".pdf".to_string(),
        pages,
        size,
        root.to_string(),
        String::new(),
        String::new(),
        0,
    ))
}

/// 章节目录漫画（folder 章节，封面由系列封面机制处理）。
/// 页数/大小由调用方从分类结果直接给出，扫描期间不再对图片做自然排序。
fn folder_chapter_comic(root: &str, rel_path: &str, page_count: i64, size: i64) -> Comic {
    let title = Path::new(rel_path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    Comic::new(
        db::generate_id(root, rel_path),
        title,
        rel_path.to_string(),
        "folder".to_string(),
        "folder".to_string(),
        page_count,
        size,
        root.to_string(),
        String::new(),
        String::new(),
        0,
    )
}

// ══════════════════════════════════════════════════════════
//  主扫描入口（递归容器识别）
// ══════════════════════════════════════════════════════════

/// 稳定 id：根目录级漫画单元（rel 为空）沿用旧版 "." 作为 key，保持已存 id 不变
fn comic_id(root_dir: &str, rel: &str) -> String {
    let key = if rel.is_empty() { "." } else { rel };
    db::generate_id(root_dir, key)
}

/// 目录相对路径 → 记录 path（根级为 "."）
fn rel_to_path(rel: &str) -> String {
    if rel.is_empty() { ".".to_string() } else { rel.to_string() }
}

/// 目录名（根级取根目录名）
fn dir_title(rel: &str, root_dir: &str) -> String {
    Path::new(rel)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            Path::new(root_dir)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| root_dir.to_string())
        })
}

/// 直接子目录中是否有含 index.json 的（容器判定）
pub(crate) fn any_child_has_index(dir: &Path) -> bool {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if is_hidden_dir(&name) || is_meta_file(&name) {
                    continue;
                }
                if path.join("index.json").is_file() {
                    return true;
                }
            }
        }
    }
    false
}

/// 目录是否为「合集/父目录型」：其下子项是独立漫画而非章节。
/// role 声明优先，缺省（旧 index.json 无 role 或没有 index.json）按结构推断。
/// 用于限制扫描单元深度：根 → 漫画本体 两层，合集套合集（如 D:\漫画 下的
/// 《神奇漫画》）视为超层，扫描时跳过不穿透，避免与单独导入的该合集根重复索引。
fn is_collection_dir(dir: &Path) -> bool {
    if let Some(meta) = load_index_meta(&dir.to_string_lossy()) {
        match meta.role.as_deref() {
            Some(ROLE_COLLECTION) => return true,
            Some(ROLE_COMIC) | Some(ROLE_SINGLE) => return false,
            _ => {} // 旧文件无 role → 按结构推断
        }
    }
    any_child_has_index(dir)
}

/// 扫描「集合」目录（role=collection 或结构推断为集合）：
/// 子项是独立漫画，父级保留为集合卡片（series），
/// 每个子项的代表记录 series_id 指向父集合，子项内部的章节保持指向子漫画。
fn scan_collection(dir: &Path, rel: &str, root_dir: &str, cache: &Option<PageCache>) -> Vec<Comic> {
    let collection_id = comic_id(root_dir, rel);
    let mut children: Vec<Comic> = Vec::new();
    let mut child_count: i64 = 0;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if is_hidden_dir(&name) || is_meta_file(&name) {
                continue;
            }
            let path = entry.path();
            let child_rel = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            if path.is_dir() {
                // 超层合集子目录（其下还有一层漫画本体，如 D:\漫画 下的《神奇漫画》）：
                // 已超出「根 → 漫画本体」两层单元深度，跳过不穿透。
                // 若该合集已被单独导入为根，内容由该根管理，避免重复索引。
                if is_collection_dir(&path) {
                    log::info!("跳过超层合集子目录（不穿透）: {}", path.display());
                    continue;
                }
                let mut sub = scan_into(&path, &child_rel, root_dir, cache);
                if !sub.is_empty() {
                    // 代表项（子漫画本身）指向父集合；其余（子漫画的章节）保持指向子漫画
                    sub[0].series_id = collection_id.clone();
                    children.push(sub.remove(0));
                    children.extend(sub);
                    child_count += 1;
                }
            } else if is_archive(&name) {
                if let Some(mut c) = scan_archive(&path, root_dir, &child_rel, cache) {
                    c.series_id = collection_id.clone();
                    children.push(c);
                    child_count += 1;
                }
            } else if is_pdf(&name) {
                if let Some(mut c) = scan_pdf(&path, root_dir, &child_rel, cache) {
                    c.series_id = collection_id.clone();
                    children.push(c);
                    child_count += 1;
                }
            }
        }
    }
    let mut collection = Comic::new(
        collection_id.clone(),
        dir_title(rel, root_dir),
        rel_to_path(rel),
        "series".to_string(),
        "folder".to_string(),
        children.iter().map(|c| c.page_count).sum(),
        children.iter().map(|c| c.size).sum(),
        root_dir.to_string(),
        String::new(),
        find_cover_file(dir).unwrap_or_default(),
        child_count,
    );
    // 集合父级不作为书架卡片展示（子漫画直接抽离到书架），仅作为子项 series_id 的锚点
    collection.is_container = true;
    let mut result = vec![collection];
    result.extend(children);
    result
}

/// 处理一个「漫画单元」目录（含有效 index.json）：系列 或 单本。
/// rel 为该目录相对 root 的路径（根级为空串）。
fn scan_comic_unit(dir: &Path, rel: &str, root_dir: &str, cache: &Option<PageCache>) -> Vec<Comic> {
    let dir_str = dir.to_string_lossy().to_string();
    // 单次遍历目录分类（图片计数/大小 + 章节条目 + 封面，一个 read_dir 完成，图片不排序）
    let c = classify_dir(dir);

    // 章节条目 = 子目录 + 压缩包 + PDF（自然排序）
    let mut chapter_names: Vec<String> =
        Vec::with_capacity(c.dirs.len() + c.archives.len() + c.pdfs.len());
    chapter_names.extend(c.dirs.iter().cloned());
    chapter_names.extend(c.archives.iter().cloned());
    chapter_names.extend(c.pdfs.iter().map(|p| {
        p.file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    }));
    chapter_names.sort_by_key(|e| db::natural_key(e));

    // 单本：无章节子项 → 目录直接含图片，或单本 PDF 文件
    if chapter_names.is_empty() {
        if !c.images.is_empty() {
            let size: i64 = c.images.iter().map(|(_, s)| *s as i64).sum();
            let comic = Comic::new(
                comic_id(root_dir, rel),
                dir_title(rel, root_dir),
                rel_to_path(rel),
                "folder".to_string(),
                "folder".to_string(),
                c.images.len() as i64,
                size,
                root_dir.to_string(),
                String::new(),
                c.cover.clone().unwrap_or_default(),
                0,
            );
            return vec![comic];
        }
        // 单本 PDF：目录内 PDF 文件
        if !c.pdfs.is_empty() {
            let size = c
                .pdfs
                .iter()
                .filter_map(|p| p.metadata().ok())
                .map(|m| m.len() as i64)
                .sum();
            let pages = cached_pages(cache, rel, size, || {
                c.pdfs.iter().map(|p| pdf_page_count(p)).sum()
            });
            let comic = Comic::new(
                comic_id(root_dir, rel),
                dir_title(rel, root_dir),
                rel_to_path(rel),
                "pdf".to_string(),
                ".pdf".to_string(),
                pages,
                size,
                root_dir.to_string(),
                String::new(),
                c.cover.clone().unwrap_or_default(),
                0,
            );
            return vec![comic];
        }
        return vec![];
    }

    // 系列：章节来自子目录/压缩包，index.json 覆盖标题
    let series_id = comic_id(root_dir, rel);
    let index_map = load_index_json(&dir_str)
        .map(|(title_map, _)| title_map)
        .unwrap_or_default();
    let mut chapters: Vec<Comic> = Vec::new();
    // 嵌套系列（子目录含 PDF）的深层章节，series_id 指向子系列本身
    let mut nested: Vec<Comic> = Vec::new();
    let mut total_pages: i64 = 0;
    let mut total_size: i64 = 0;

    for name in &chapter_names {
        let entry_path = dir.join(name);
        let ch_rel = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
        let mut ch: Option<Comic> = None;
        if is_archive(name) {
            ch = scan_archive(&entry_path, root_dir, &ch_rel, cache);
        } else if is_pdf(name) {
            ch = scan_pdf(&entry_path, root_dir, &ch_rel, cache);
        } else if entry_path.is_dir() {
            let sub = classify_dir(&entry_path);
            if !sub.images.is_empty() {
                // 章节目录：直接含图片（大小取自枚举缓存，无额外 stat）
                let size: i64 = sub.images.iter().map(|(_, s)| *s as i64).sum();
                ch = Some(folder_chapter_comic(root_dir, &ch_rel, sub.images.len() as i64, size));
            } else if !sub.pdfs.is_empty() || entry_path.join("index.json").is_file() {
                // 子目录是独立漫画单元（单本 pdf / 子系列 / 含 index.json 的漫画）：
                // 递归识别，取代表项作为本章节，其余（子系列的章节）并入结果
                let mut sub_units = scan_into(&entry_path, &ch_rel, root_dir, cache);
                if !sub_units.is_empty() {
                    ch = Some(sub_units.remove(0));
                    nested.extend(sub_units);
                }
            }
        }
        if let Some(mut c) = ch {
            c.series_id = series_id.clone();
            if let Some(t) = index_map.get(name) {
                c.title = t.clone();
            }
            total_pages += c.page_count;
            total_size += c.size;
            chapters.push(c);
        }
    }

    let series = Comic::new(
        series_id.clone(),
        dir_title(rel, root_dir),
        rel_to_path(rel),
        "series".to_string(),
        "folder".to_string(),
        total_pages,
        total_size,
        root_dir.to_string(),
        String::new(),
        c.cover.clone().unwrap_or_default(),
        chapters.len() as i64,
    );

    let mut result = vec![series];
    result.extend(chapters);
    result.extend(nested);
    result
}

/// 递归扫描：识别集合 / 漫画本体 / 单本。
/// 单元深度限制为两层（根 → 漫画本体）：
///   - 根 = 漫画本体（其下是章节）或 漫画本体的父目录（其下是平级漫画）
///   - 集合子项若仍是合集（合集套合集）→ 视为超层，跳过不穿透（见 is_collection_dir）
/// 判定规则（index.json 的 role 声明优先，缺省按结构推断）：
///   - role=collection → 集合：子项是独立漫画，父级保留为集合卡片
///   - role=comic / single → 漫画单元（系列 / 单本）
///   - 无 role 的旧 index.json → 按结构推断（子项含 index.json → collection）
///   - 无 index.json 的旧式单本/系列 → 自动补齐 index.json（含推断 role）后重扫
///   - 其余（无 index.json 且无漫画结构）→ 跳过
fn scan_into(dir: &Path, rel: &str, root_dir: &str, cache: &Option<PageCache>) -> Vec<Comic> {
    let dir_str = dir.to_string_lossy().to_string();

    if dir.join("index.json").is_file() {
        let role = load_index_meta(&dir_str).and_then(|m| m.role);
        match role.as_deref() {
            Some(ROLE_COLLECTION) => return scan_collection(dir, rel, root_dir, cache),
            Some(ROLE_COMIC) | Some(ROLE_SINGLE) => {
                return scan_comic_unit(dir, rel, root_dir, cache)
            }
            _ => {
                // 旧文件无 role 声明：按结构推断，并补写 role（一次性迁移）
                if infer_role(dir) == ROLE_COLLECTION {
                    write_role_if_missing(&dir_str, ROLE_COLLECTION);
                    return scan_collection(dir, rel, root_dir, cache);
                }
                write_role_if_missing(&dir_str, ROLE_COMIC);
                return scan_comic_unit(dir, rel, root_dir, cache);
            }
        }
    }

    // 无 index.json：子项含 index.json → 是集合，补齐 role 后重扫
    if any_child_has_index(dir) {
        ensure_index_json(&dir_str);
        return scan_into(dir, rel, root_dir, cache);
    }

    let chapter_entries = collect_chapter_entries(&dir_str);
    if chapter_entries.is_empty() {
        // 旧式单本：直接含图片 → 补齐 index.json（role=single）后重扫
        if !classify_dir(dir).images.is_empty() {
            ensure_index_json(&dir_str);
            return scan_into(dir, rel, root_dir, cache);
        }
        return vec![];
    }
    // 旧式系列：补齐 index.json（role 按结构推断）后重扫
    ensure_index_json(&dir_str);
    scan_into(dir, rel, root_dir, cache)
}

/// 扫描一个根目录，返回识别出的全部记录。
/// 根只允许两种形态：漫画本体（其下是章节），或漫画本体的父目录（其下是平级漫画）。
/// 集合子目录套集合（如父目录下的合集）超出两层，扫描时跳过不穿透。
pub fn scan_directory(root_dir: &str) -> Result<Vec<Comic>, String> {
    let root = Path::new(root_dir);
    if !root.is_dir() {
        return Err(format!("目录不存在: {root_dir}"));
    }
    // 增量页数缓存：DB 已有记录（PDF/压缩包）且文件未变 → 跳过重复解析
    let cache = db::comics::load_root_comics(root_dir)
        .ok()
        .map(|comics| {
            let mut m = PageCache::new();
            for c in &comics {
                if c.kind == "pdf" || c.kind == "archive" {
                    m.insert((c.path.clone(), c.size), c.page_count);
                }
            }
            m
        });
    Ok(scan_into(root, "", root_dir, &cache))
}

// ══════════════════════════════════════════════════════════
//  页面列表（供阅读器复用）
// ══════════════════════════════════════════════════════════

/// 列出漫画的页面文件名（folder 目录 / zip 内部条目），自然排序，排除 cover.*
pub fn list_page_files(comic: &Comic) -> Vec<String> {
    let full_path = Path::new(&comic.root_dir).join(&comic.path);
    match comic.kind.as_str() {
        "folder" => {
            let mut names: Vec<String> = std::fs::read_dir(&full_path)
                .map(|entries| {
                    entries
                        .flatten()
                        .filter(|e| e.path().is_file())
                        .map(|e| e.file_name().to_string_lossy().to_string())
                        .filter(|n| is_image_file(n) && !is_meta_file(n))
                        .collect()
                })
                .unwrap_or_default();
            let sort_key = |n: &String| {
                let stem = Path::new(n)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                db::natural_key(&stem)
            };
            names.sort_by_key(sort_key);
            names
        }
        "archive" => {
            let Ok(file) = std::fs::File::open(&full_path) else {
                return vec![];
            };
            let Ok(archive) = zip::ZipArchive::new(file) else {
                return vec![];
            };
            let mut names: Vec<String> = archive
                .file_names()
                .filter(|n| !n.ends_with('/') && is_image_file(n) && !is_meta_file(n))
                .map(|s| s.to_string())
                .collect();
            let sort_key = |n: &String| {
                let stem = Path::new(n)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                db::natural_key(&stem)
            };
            names.sort_by_key(sort_key);
            names
        }
        _ => vec![],
    }
}

/// 读取指定页面，返回 (字节, mime)
pub fn read_page(comic: &Comic, page_index: usize) -> Option<(Vec<u8>, String)> {
    let files = list_page_files(comic);
    let name = files.get(page_index)?.clone();
    read_page_named(comic, &name)
}

/// 按文件名读取页面（folder 目录 / zip 内部条目），返回 (字节, mime)
pub fn read_page_named(comic: &Comic, name: &str) -> Option<(Vec<u8>, String)> {
    let full_path = Path::new(&comic.root_dir).join(&comic.path);
    let mime = mime_for(name);
    match comic.kind.as_str() {
        "folder" => {
            let bytes = std::fs::read(full_path.join(name)).ok()?;
            Some((bytes, mime))
        }
        "archive" => {
            let file = std::fs::File::open(&full_path).ok()?;
            let mut archive = zip::ZipArchive::new(file).ok()?;
            let mut entry = archive.by_name(name).ok()?;
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut bytes).ok()?;
            Some((bytes, mime))
        }
        _ => None,
    }
}

pub fn mime_for(name: &str) -> String {
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "tiff" | "tif" => "image/tiff",
        "avif" => "image/avif",
        _ => "application/octet-stream",
    }
    .to_string()
}
