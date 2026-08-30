//! 故事会封面库：按年份拆分的小型 SQLite，存于各年份目录（数据跟内容走，可随目录整体迁移）
//!
//! 布局：`{root}/{year}/storyclub-covers.db`（每年一个库）；未分类期数（year 空串）存根库 `{root}/storyclub-covers.db`。
//! 读取时按期数的年份打开对应库，各自独立、互不影响。
//!
//! 封面统一降采样到 320px 后入库（书架展示仅需几百 px，避免原图撑爆库导致加载卡顿）：
//!  - 新写入（后台补齐/阅读器）先压缩；
//!  - 存量旧库（分库前单一 90M+ 大库）由一次性后台任务迁移到各年库并压缩，读取时也会惰性压缩；
//!  - 旧版散落文件（PDF 旁 `{stem}.story.jpg` / 旧公共目录）读取时惰性迁移入库并删除原文件。
use crate::db::storyclub as story_db;
use base64::Engine as _;
use image::GenericImageView;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// 封面库文件名（位于各年份目录 / 根目录下）
const DB_NAME: &str = "storyclub-covers.db";
/// 超过该字节数视为未压缩原图（320px JPEG 一般 <30KB），读取时重新降采样
const LARGE_THRESHOLD: usize = 40_000;
/// 封面目标宽度（与前端 pdf.js 渲染一致，保证书架加载流畅）
const COVER_WIDTH: u32 = 320;

static COMPRESS_RUNNING: AtomicBool = AtomicBool::new(false);

fn root_dir() -> Result<PathBuf, String> {
    let root = story_db::get_root()?.ok_or_else(|| "尚未设置故事会目录".to_string())?;
    Ok(Path::new(&root).to_path_buf())
}

/// 某年份的封面库路径（year 空串 → 根库）
fn cover_db_path(year: &str) -> Result<PathBuf, String> {
    let root = root_dir()?;
    Ok(if year.is_empty() {
        root.join(DB_NAME)
    } else {
        root.join(year).join(DB_NAME)
    })
}

/// 分库前的旧单一封面库路径（= 根库；未分类期数也用它）
fn root_db_path() -> Result<PathBuf, String> {
    cover_db_path("")
}

fn open_at(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(|e| format!("打开封面库失败: {e}"))?;
    // 后台迁移/补齐多路并发写库，设置忙等待避免偶发 "database is locked"
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|e| format!("设置封面库超时失败: {e}"))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS covers (
            issue_id TEXT PRIMARY KEY,
            data BLOB NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    )
    .map_err(|e| format!("初始化封面库失败: {e}"))?;
    Ok(conn)
}

fn open_for(year: &str) -> Result<Connection, String> {
    open_at(&cover_db_path(year)?)
}

fn jpeg_data_url(bytes: &[u8]) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("data:image/jpeg;base64,{b64}")
}

fn query(conn: &Connection, issue_id: &str) -> Result<Option<Vec<u8>>, String> {
    let mut stmt = conn
        .prepare("SELECT data FROM covers WHERE issue_id = ?1")
        .map_err(|e| format!("准备查询失败: {e}"))?;
    stmt.query_row(params![issue_id], |r| r.get(0))
        .optional()
        .map_err(|e| format!("读取封面失败: {e}"))
}

fn upsert_with(conn: &Connection, issue_id: &str, data: &[u8]) -> Result<(), String> {
    conn.execute(
        "INSERT INTO covers(issue_id, data, updated_at) VALUES(?1, ?2, datetime('now'))
         ON CONFLICT(issue_id) DO UPDATE SET data = excluded.data, updated_at = excluded.updated_at",
        params![issue_id, data],
    )
    .map_err(|e| format!("保存封面失败: {e}"))?;
    Ok(())
}

/// JPEG 宽度超过 target_width 时等比降采样重编码（小图或解码/编码异常返回 None，调用方用原图）
pub(crate) fn downscale_jpeg_to(bytes: &[u8], target_width: u32) -> Option<Vec<u8>> {
    let img = image::load_from_memory(bytes).ok()?;
    let (w, h) = img.dimensions();
    if w <= target_width {
        return None;
    }
    let nw = target_width.max(1);
    let nh = (h as u64 * nw as u64 / w as u64).max(1) as u32;
    let small = img.resize(nw, nh, image::imageops::FilterType::Triangle);
    let mut out = Vec::new();
    small
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Jpeg)
        .ok()?;
    Some(out)
}

/// 超过阈值的大图就地压缩写回并返回小图；无法压缩的异常图原样返回（不写回，避免反复尝试）
fn ensure_small(conn: &Connection, issue_id: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() <= LARGE_THRESHOLD {
        return Ok(bytes.to_vec());
    }
    match downscale_jpeg_to(bytes, COVER_WIDTH) {
        Some(small) => {
            upsert_with(conn, issue_id, &small)?;
            Ok(small)
        }
        None => Ok(bytes.to_vec()),
    }
}

/// 解析单期封面：目标（该年）库 → 旧根库（分库前单一库）惰性迁移 → 旧散落文件惰性迁移。
/// `conn` 为目标年份库连接；均无返回 None。
fn resolve_cover(conn: &Connection, year: &str, issue_id: &str) -> Result<Option<Vec<u8>>, String> {
    if let Some(b) = query(conn, issue_id)? {
        return Ok(Some(ensure_small(conn, issue_id, &b)?));
    }
    // 分库前的旧单一库（根库）惰性迁移到该年库并压缩
    if !year.is_empty() {
        if let Ok(lc) = Connection::open(root_db_path()?) {
            if let Some(b) = query(&lc, issue_id)? {
                let small = ensure_small(&lc, issue_id, &b)?;
                upsert_with(conn, issue_id, &small)?;
                let _ = lc.execute("DELETE FROM covers WHERE issue_id = ?1", params![issue_id]);
                return Ok(Some(small));
            }
        }
    }
    // 旧散落封面文件（PDF 旁 / 公共目录）惰性迁移
    if let Some(b) = migrate_legacy(issue_id)? {
        let small = ensure_small(conn, issue_id, &b)?;
        return Ok(Some(small));
    }
    Ok(None)
}

/// 单期封面字节（已统一压缩到 320px）
pub fn get_cover(issue_id: &str) -> Result<Option<Vec<u8>>, String> {
    let year = story_db::get_issue(issue_id)?
        .map(|i| i.year)
        .unwrap_or_default();
    let conn = open_for(&year)?;
    resolve_cover(&conn, &year, issue_id)
}

/// 批量取封面 data URL：按年份分组开库，逐期查询（缺失时惰性迁移旧库/旧文件），仅返回命中期
pub fn get_covers_batch(issue_ids: &[String]) -> Result<Vec<(String, String)>, String> {
    let issues = story_db::list_issues()?;
    let year_of: HashMap<&str, &str> = issues
        .iter()
        .map(|i| (i.id.as_str(), i.year.as_str()))
        .collect();
    let mut buckets: HashMap<&str, Vec<&String>> = HashMap::new();
    for id in issue_ids {
        let y = year_of.get(id.as_str()).copied().unwrap_or("");
        buckets.entry(y).or_default().push(id);
    }
    let mut out = Vec::new();
    for (year, ids) in buckets {
        let conn = open_for(year)?;
        for id in ids {
            if let Some(bytes) = resolve_cover(&conn, year, id)? {
                out.push((id.clone(), jpeg_data_url(&bytes)));
            }
        }
    }
    Ok(out)
}

/// 写入/更新期数封面（先压缩到 320px；按期数年份落到对应库）
pub fn upsert(issue_id: &str, data: &[u8]) -> Result<(), String> {
    let small = match downscale_jpeg_to(data, COVER_WIDTH) {
        Some(s) => s,
        None => data.to_vec(),
    };
    let year = story_db::get_issue(issue_id)?
        .map(|i| i.year)
        .unwrap_or_default();
    let conn = open_for(&year)?;
    upsert_with(&conn, issue_id, &small)
}

/// 迁移旧版散落封面文件到库（PDF 旁 `{stem}.story.jpg` → 旧公共目录），读后删除原文件。
/// 无旧文件返回 None。
fn migrate_legacy(issue_id: &str) -> Result<Option<Vec<u8>>, String> {
    let Some(issue) = story_db::get_issue(issue_id)? else {
        return Ok(None);
    };
    let mut candidate: Option<PathBuf> = {
        let p = Path::new(&issue.path);
        let stem = p
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| issue.id.clone());
        let side = p.with_file_name(format!("{stem}.story.jpg"));
        if side.is_file() {
            Some(side)
        } else {
            None
        }
    };
    if candidate.is_none() {
        let legacy = crate::app_data_dir()
            .join("storyclub_covers")
            .join(format!("{}.jpg", issue.id));
        if legacy.is_file() {
            candidate = Some(legacy);
        }
    }
    let Some(path) = candidate else { return Ok(None) };
    let bytes = std::fs::read(&path).map_err(|e| format!("读取旧封面失败: {e}"))?;
    let _ = std::fs::remove_file(&path);
    Ok(Some(bytes))
}

/// 清理各库中已不存在的期数记录（换根/重扫后防孤儿数据膨胀）
pub fn prune_orphans(valid_ids: &[String]) -> Result<(), String> {
    let root = root_dir()?;
    let valid: HashSet<&String> = valid_ids.iter().collect();
    // 根库 + 各年份目录库
    let mut dbs = vec![root.join(DB_NAME)];
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                dbs.push(entry.path().join(DB_NAME));
            }
        }
    }
    for db in dbs {
        if !db.is_file() {
            continue;
        }
        let conn = open_at(&db)?;
        let mut stmt = conn
            .prepare("SELECT issue_id FROM covers")
            .map_err(|e| format!("准备查询失败: {e}"))?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| format!("查询封面失败: {e}"))?;
        for row in rows {
            let id = row.map_err(|e| format!("读取封面失败: {e}"))?;
            if !valid.contains(&id) {
                conn.execute("DELETE FROM covers WHERE issue_id = ?1", params![id])
                    .map_err(|e| format!("清理封面失败: {e}"))?;
            }
        }
    }
    Ok(())
}

// ══════════════════════════════════════════════════════════
//  一次性后台任务：分库前的旧单一库 → 迁移到各年库并压缩
// ══════════════════════════════════════════════════════════

/// 启动后台迁移压缩（已在跑则忽略；扫描/进入书架时调用）
pub fn start_migrate_and_compress() {
    if COMPRESS_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let result = migrate_and_compress_legacy();
        COMPRESS_RUNNING.store(false, Ordering::SeqCst);
        if let Err(e) = result {
            log::warn!("故事会封面库迁移压缩失败: {e}");
        }
    });
}

fn migrate_and_compress_legacy() -> Result<(), String> {
    // 单次任务最多迁移 150 期：每张原图需解码+重编码（CPU 重活），
    // 分批分散到多次书架访问逐步收敛旧大库，避免一次跑完占满 CPU 拖卡页面。
    // 任务可续（迁走的行已从旧库删除），下次进入书架继续。
    const BUDGET: usize = 150;
    const SLEEP_MS: u64 = 20;
    let root_db = root_db_path()?;
    if !root_db.is_file() {
        return Ok(());
    }
    let lc = open_at(&root_db)?;
    // 旧库全部行
    let mut stmt = lc
        .prepare("SELECT issue_id, data FROM covers")
        .map_err(|e| format!("准备查询失败: {e}"))?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)))
        .map_err(|e| format!("查询封面失败: {e}"))?;
    let rows: Vec<(String, Vec<u8>)> = rows
        .collect::<Result<_, _>>()
        .map_err(|e| format!("读取封面失败: {e}"))?;
    if rows.is_empty() {
        let _ = std::fs::remove_file(&root_db);
        return Ok(());
    }
    // id → 年份
    let year_of: HashMap<String, String> = story_db::list_issues()?
        .into_iter()
        .map(|i| (i.id, i.year))
        .collect();
    for (idx, (id, data)) in rows.iter().enumerate() {
        if idx >= BUDGET {
            break;
        }
        let year = year_of.get(id).cloned().unwrap_or_default();
        if year.is_empty() {
            // 未分类期数：留在根库，仅压缩
            let _ = ensure_small(&lc, id, data);
        } else {
            let small = match downscale_jpeg_to(data, COVER_WIDTH) {
                Some(s) => s,
                None => data.clone(),
            };
            let conn = open_for(&year)?;
            upsert_with(&conn, id, &small)?;
            let _ = lc.execute("DELETE FROM covers WHERE issue_id = ?1", params![id]);
        }
        // 逐期短暂喘息，避免持续 CPU/磁盘占用影响书架浏览
        std::thread::sleep(Duration::from_millis(SLEEP_MS));
    }
    // 根库已无剩余记录 → 删除旧大库文件
    let remaining: i64 = lc
        .query_row("SELECT COUNT(*) FROM covers", [], |r| r.get(0))
        .map_err(|e| format!("统计封面失败: {e}"))?;
    if remaining == 0 {
        let _ = std::fs::remove_file(&root_db);
    }
    Ok(())
}

// ══════════════════════════════════════════════════════════
//  后台封面补齐：后端独立线程静默生成缺封面期数（服务端生成，
//  前端只展示 + 监听进度事件；刷新页面不再重启客户端逐 PDF 下载）
// ══════════════════════════════════════════════════════════

/// 轻量校验 PDF 可读性：头部 %PDF 魔数 + 尾部 startxref 标记。
/// 封面补齐前过滤损坏文件，避免对每期反复整读等待超时。
pub(crate) fn pdf_quick_check(path: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = [0u8; 1024];
    if f.read(&mut head).unwrap_or(0) < 8 {
        return false;
    }
    if &head[..5] != b"%PDF-" {
        return false;
    }
    let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if len < 2048 {
        return len >= 512;
    }
    let tail_len = 2048usize.min(len as usize);
    let mut tail = vec![0u8; tail_len];
    if f.seek(SeekFrom::End(-(tail_len as i64))).is_err() || f.read_exact(&mut tail).is_err() {
        return false;
    }
    String::from_utf8_lossy(&tail).contains("startxref")
}

/// 取文档第一页对象 id：手动遍历页面树。
/// 不用 lopdf 的 `get_pages()`——它用 `get()` 读 `/Kids` 不自动解引用，
/// 对 `/Kids` 为间接引用（`/Kids 4 0 R`）的 PDF 会返回空页（文件本身正常，pdf.js 阅读正常）。
fn first_page_id(doc: &lopdf::Document) -> Result<lopdf::ObjectId, String> {
    let pages_ref = doc
        .catalog()
        .map_err(|e| format!("读取文档目录失败: {e}"))?
        .get(b"Pages")
        .and_then(|o| o.as_reference())
        .map_err(|_| "文档目录缺 Pages".to_string())?;
    let mut stack = vec![pages_ref];
    let mut visited: HashSet<lopdf::ObjectId> = HashSet::new();
    while let Some(id) = stack.pop() {
        if !visited.insert(id) {
            continue;
        }
        let dict = doc.get_dictionary(id).map_err(|e| format!("读取页面树节点失败: {e}"))?;
        // 有 Kids → 页面树中间节点（Pages），递归深入；叶子即页面对象
        if let Ok(kids) = dict.get_deref(b"Kids", doc).and_then(|o| o.as_array()) {
            for k in kids.iter().rev() {
                if let Ok(kid_id) = k.as_reference() {
                    stack.push(kid_id);
                }
            }
            continue;
        }
        // 无 Kids → 页面对象（第一页）
        return Ok(id);
    }
    Err("PDF 没有页面".to_string())
}

/// 递归收集页面 Resources.XObject 中的全部图片对象 id（Form XObject 嵌套展开）。
/// lopdf 的 `get_page_images` 只收集页面直接 Image XObject，跳过 Form——这些扫描件
/// 第一页常只有一个 Fm0（Form）包裹封面图，必须递归展开才能取到。
fn collect_image_ids(doc: &lopdf::Document, page_id: lopdf::ObjectId) -> Result<Vec<lopdf::ObjectId>, String> {
    fn walk(
        doc: &lopdf::Document,
        xobject: &lopdf::Dictionary,
        out: &mut Vec<lopdf::ObjectId>,
        visited: &mut HashSet<lopdf::ObjectId>,
    ) -> Result<(), String> {
        for (_, xvalue) in xobject.iter() {
            let id = match xvalue.as_reference() {
                Ok(id) => id,
                // 极少数内联对象：直接跳（图片必为间接引用）
                Err(_) => continue,
            };
            if !visited.insert(id) {
                continue;
            }
            let Ok(xobj) = doc.get_object(id) else { continue };
            let Ok(stream) = xobj.as_stream() else { continue };
            let subtype = stream
                .dict
                .get(b"Subtype")
                .ok()
                .and_then(|o| o.as_name().ok())
                .unwrap_or_default();
            match subtype {
                b"Image" => out.push(id),
                b"Form" => {
                    if let Ok(res) = stream.dict.get_deref(b"Resources", doc) {
                        if let Ok(rd) = res.as_dict() {
                            walk(doc, rd, out, visited)?;
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    let page = doc.get_dictionary(page_id).map_err(|e| format!("读取页面失败: {e}"))?;
    let resources = doc.get_dict_in_dict(page, b"Resources").map_err(|e| format!("读取页面资源失败: {e}"))?;
    let xobject = doc.get_dict_in_dict(resources, b"XObject").map_err(|e| format!("读取 XObject 失败: {e}"))?;
    let mut out = Vec::new();
    let mut visited = HashSet::new();
    walk(doc, xobject, &mut out, &mut visited)?;
    Ok(out)
}

/// 后端提取第一页 JPEG 作为封面：全量打开 PDF（lopdf 整读，最稳妥）。
/// 加密 PDF（空用户密码）自动解密流数据；再降采样到封面宽度。
/// 第一页无整页 JPEG 图片或解析/解密异常时返回 Err（调用方跳过该期，不产生空封面）。
pub fn first_page_jpeg_bytes(path: &Path) -> Result<Vec<u8>, String> {
    let doc = lopdf::Document::load(path).map_err(|e| format!("解析 PDF 失败: {e}"))?;
    // 加密 PDF：算出文件级密钥（空用户密码）与 AES 标记，逐图解密
    let encryption = doc_encryption(&doc)?;
    let first_page_id = first_page_id(&doc)?;
    let image_ids = collect_image_ids(&doc, first_page_id)?;
    for img_id in image_ids {
        let obj = doc.get_object(img_id).map_err(|e| format!("读取图片对象失败: {e}"))?;
        let stream = obj.as_stream().map_err(|e| format!("图片非流对象: {e}"))?;
        let dict = &stream.dict;
        // Filter 含 DCTDecode 才是 JPEG（可带 FlateDecode 预处理等复合滤镜）
        let is_jpeg = match dict.get(b"Filter") {
            Ok(lopdf::Object::Name(n)) => n == b"DCTDecode",
            Ok(lopdf::Object::Array(a)) => a
                .iter()
                .any(|o| matches!(o, lopdf::Object::Name(n) if n == b"DCTDecode")),
            _ => false,
        };
        if !is_jpeg {
            continue;
        }
        let content = match &encryption {
            Some((key, aes)) => {
                let obj = doc.get_object(img_id).map_err(|e| format!("读取图片对象失败: {e}"))?;
                lopdf::encryption::decrypt_object(key, img_id, obj, *aes)
                    .map_err(|e| format!("解密图片流失败: {e}"))?
            }
            None => stream.content.to_vec(),
        };
        if let Some(small) = downscale_jpeg_to(&content, COVER_WIDTH) {
            return Ok(small);
        }
        // 解码失败（损坏/非标准 JPEG）→ 校验原始流，无效则跳过找下一张，
        // 绝不把无法解码的字节原样入库（否则前端显示空白图）
        if image::load_from_memory(&content).is_ok() {
            return Ok(content);
        }
    }
    Err("第一页不是整页 JPEG 图片".to_string())
}

/// 取文档第 idx 页（0 起）对象 id：手动遍历页面树（DFS 顺序即文档顺序）。
/// 不用 lopdf 的 `get_pages()`（原因见 first_page_id）。
fn nth_page_id(doc: &lopdf::Document, idx: usize) -> Result<lopdf::ObjectId, String> {
    let pages_ref = doc
        .catalog()
        .map_err(|e| format!("读取文档目录失败: {e}"))?
        .get(b"Pages")
        .and_then(|o| o.as_reference())
        .map_err(|_| "文档目录缺 Pages".to_string())?;
    let mut stack = vec![pages_ref];
    let mut visited: HashSet<lopdf::ObjectId> = HashSet::new();
    let mut leaf = 0usize;
    while let Some(id) = stack.pop() {
        if !visited.insert(id) {
            continue;
        }
        let dict = doc.get_dictionary(id).map_err(|e| format!("读取页面树节点失败: {e}"))?;
        // 有 Kids → 页面树中间节点（Pages），递归深入；叶子即页面对象
        if let Ok(kids) = dict.get_deref(b"Kids", doc).and_then(|o| o.as_array()) {
            for k in kids.iter().rev() {
                if let Ok(kid_id) = k.as_reference() {
                    stack.push(kid_id);
                }
            }
            continue;
        }
        if leaf == idx {
            return Ok(id);
        }
        leaf += 1;
    }
    Err("页码越界".to_string())
}

/// 后端提取第 idx 页（0 起）JPEG 作为手机端逐页图片流：seek 式优先（毫秒级），
/// 失败回退 lopdf 整读（最稳妥，含加密）。页面原图直接返回（不降采样，保持清晰）。
pub fn page_jpeg_bytes(path: &Path, idx: usize) -> Result<Vec<u8>, String> {
    if let Ok(bytes) = crate::story_first_page::page_jpeg_stream(path, idx) {
        return Ok(bytes);
    }
    let doc = lopdf::Document::load(path).map_err(|e| format!("解析 PDF 失败: {e}"))?;
    // 加密 PDF：算出文件级密钥（空用户密码）与 AES 标记，逐图解密
    let encryption = doc_encryption(&doc)?;
    let page_id = nth_page_id(&doc, idx)?;
    let image_ids = collect_image_ids(&doc, page_id)?;
    for img_id in image_ids {
        let obj = doc.get_object(img_id).map_err(|e| format!("读取图片对象失败: {e}"))?;
        let stream = obj.as_stream().map_err(|e| format!("图片非流对象: {e}"))?;
        let dict = &stream.dict;
        // Filter 含 DCTDecode 才是 JPEG（可带 FlateDecode 预处理等复合滤镜）
        let is_jpeg = match dict.get(b"Filter") {
            Ok(lopdf::Object::Name(n)) => n == b"DCTDecode",
            Ok(lopdf::Object::Array(a)) => a
                .iter()
                .any(|o| matches!(o, lopdf::Object::Name(n) if n == b"DCTDecode")),
            _ => false,
        };
        if !is_jpeg {
            continue;
        }
        let content = match &encryption {
            Some((key, aes)) => {
                let obj = doc.get_object(img_id).map_err(|e| format!("读取图片对象失败: {e}"))?;
                lopdf::encryption::decrypt_object(key, img_id, obj, *aes)
                    .map_err(|e| format!("解密图片流失败: {e}"))?
            }
            None => stream.content.to_vec(),
        };
        // 校验可解码（损坏/非标准 JPEG 直接返回会白屏），页面原图不降采样
        if image::load_from_memory(&content).is_ok() {
            return Ok(content);
        }
    }
    Err(format!("第 {} 页不是整页 JPEG 图片", idx + 1))
}

/// 加密 PDF 的文件级密钥（空用户密码）与 AES 模式；未加密返回 None。
/// 复用 lopdf 自带的标准安全处理器（RC4/AES 均支持）。
fn doc_encryption(doc: &lopdf::Document) -> Result<Option<(Vec<u8>, bool)>, String> {
    let enc_id = match doc.trailer.get(b"Encrypt") {
        Ok(o) => o.as_reference().map_err(|_| "Encrypt 非引用".to_string())?,
        Err(_) => return Ok(None),
    };
    let key = lopdf::encryption::get_encryption_key(doc, &[] as &[u8], true)
        .map_err(|e| format!("PDF 解密失败（可能设置了密码）: {e}"))?;
    let enc = doc.get_object(enc_id).map_err(|e| format!("读取加密字典失败: {e}"))?;
    let dict = enc.as_dict().map_err(|_| "Encrypt 非字典".to_string())?;
    let aes = dict
        .get(b"CF")
        .ok()
        .and_then(|o| o.as_dict().ok())
        .and_then(|d| d.get(b"StdCF").ok())
        .and_then(|o| o.as_dict().ok())
        .and_then(|d| d.get(b"CFM").ok())
        .and_then(|o| o.as_name().ok())
        .map(|n| n == b"AESV2")
        .unwrap_or(false);
    Ok(Some((key, aes)))
}

/// 尚无封面的期数 id 列表（缺封面 = 目标年库 / 旧根库 / 旧散落文件均无）。
/// 各库一次查询全部 id（避免逐期开库连接），旧散落文件用文件存在性判断。
pub fn missing_cover_ids() -> Result<Vec<String>, String> {
    let issues = story_db::list_issues()?;
    let mut present: HashSet<String> = HashSet::new();
    let root = root_dir()?;
    // 各年份目录库 + 根库（未分类期数），每库一次查询全部封面 id
    let mut dbs: Vec<PathBuf> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                dbs.push(p.join(DB_NAME));
            }
        }
    }
    dbs.push(root.join(DB_NAME));
    for db in dbs {
        if !db.is_file() {
            continue;
        }
        let Ok(conn) = open_at(&db) else { continue };
        let Ok(mut stmt) = conn.prepare("SELECT issue_id FROM covers") else { continue };
        let ids: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .and_then(|rows| rows.collect())
            .unwrap_or_default();
        present.extend(ids);
    }
    let legacy_dir = crate::app_data_dir().join("storyclub_covers");
    let mut missing = Vec::new();
    for i in issues {
        if present.contains(&i.id) {
            continue;
        }
        // 旧散落封面文件（PDF 旁 / 旧公共目录）视为已有封面
        let stem = Path::new(&i.path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| i.id.clone());
        if Path::new(&i.path).with_file_name(format!("{stem}.story.jpg")).is_file() {
            continue;
        }
        if legacy_dir.join(format!("{}.jpg", i.id)).is_file() {
            continue;
        }
        missing.push(i.id);
    }
    Ok(missing)
}


