//! 漫画本体目录级封面库（唯一来源）
//!
//! 在每个「漫画本体」目录下维护一个 `.leisure-covers.db`，统一保存该漫画
//! 本体及其全部章节的封面。key 为记录相对其本体目录的路径（`rel_key_of`），
//! 每个本体库内唯一：只随本体目录内部结构变化，不依赖根目录层级，
//! 因此更换/调整根目录不会导致封面失配。封面跟着漫画目录走：整体拷贝某个
//! 漫画目录到别处后，重新导入即可直接复用，手动设置的封面不丢失。
//! 应用库（leisure.db）不再存封面。
//!
//! 归属规则：collection（is_container）不是漫画本体；series 若父为 collection
//! 或无父即为本体；嵌套 series 与章节归到最近的漫画本体；顶级单本（无父的
//! folder/archive/pdf）以自身目录（文件则为父目录）为本体目录。
//!
//! 可靠性约定：封面是纯缓存、可再生成，因此外部库所有操作失败一律由
//! 调用方降级（读取视为未命中、写入静默忽略），绝不阻塞主流程。
use crate::db::comics::Comic;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// 封面库文件名（位于漫画本体目录下；非图片/压缩包/PDF 扩展名，扫描器自动忽略）
pub const COVER_DB_FILE: &str = ".leisure-covers.db";

/// 本体目录 → 已打开的外部库连接（SQLite 连接非 Sync，全局锁串行化访问）
static CONNS: Mutex<Option<HashMap<String, Connection>>> = Mutex::new(None);
/// comic_id → 漫画本体目录（绝对路径）归属缓存；扫描/删除后失效重建
static BUCKETS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

/// 封面库完整路径（本体目录下）
pub fn cover_db_path(bucket_dir: &str) -> PathBuf {
    Path::new(bucket_dir).join(COVER_DB_FILE)
}

/// 归属缓存失效（扫描/删除记录后调用，下次请求时重建）
pub fn invalidate_buckets() {
    if let Ok(mut g) = BUCKETS.lock() {
        *g = None;
    }
}

/// 计算某记录所属的漫画本体目录（绝对路径）；collection 返回 None
pub fn bucket_dir(comic: &Comic) -> Option<String> {
    buckets().get(&comic.id).cloned()
}

/// 封面库 key：记录相对其本体目录的路径（不依赖根目录层级）。
/// 本体目录自身返回空串；章节/单本返回相对本体目录的子路径。
pub fn rel_key_of(comic: &Comic, bucket: &str) -> String {
    let abs = Path::new(&comic.root_dir).join(&comic.path);
    abs.strip_prefix(Path::new(bucket))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// 全部记录 → 漫画本体目录的归属映射（构建一次并缓存）
pub fn all_buckets() -> HashMap<String, String> {
    buckets()
}

fn buckets() -> HashMap<String, String> {
    if let Ok(guard) = BUCKETS.lock() {
        if let Some(m) = &*guard {
            return m.clone();
        }
    }
    let map = build_buckets();
    if let Ok(mut g) = BUCKETS.lock() {
        *g = Some(map.clone());
    }
    map
}

fn build_buckets() -> HashMap<String, String> {
    let all = match crate::db::comics::load_all_comics() {
        Ok(a) => a,
        Err(_) => return HashMap::new(),
    };
    let by_id: HashMap<String, Comic> = all.into_iter().map(|c| (c.id.clone(), c)).collect();
    let ids: Vec<String> = by_id.keys().cloned().collect();
    let mut map = HashMap::new();
    for id in ids {
        if let Some(c) = by_id.get(&id) {
            if let Some(b) = bucket_of(&by_id, c) {
                map.insert(id, b);
            }
        }
    }
    map
}

/// 归属规则：递归向上找最近的漫画本体
fn bucket_of(by_id: &HashMap<String, Comic>, comic: &Comic) -> Option<String> {
    let root = &comic.root_dir;
    // collection：容器本身不是漫画本体
    if comic.is_container {
        return None;
    }
    if comic.kind == "series" {
        // 无父（顶级系列）或父为 collection（子系列）→ 本体是自己
        if comic.series_id.is_empty() {
            return Some(Path::new(root).join(&comic.path).to_string_lossy().to_string());
        }
        if let Some(p) = by_id.get(&comic.series_id) {
            if p.is_container {
                return Some(Path::new(root).join(&comic.path).to_string_lossy().to_string());
            }
            return bucket_of(by_id, p);
        }
        return Some(Path::new(root).join(&comic.path).to_string_lossy().to_string());
    }
    // 章节/单本
    if comic.series_id.is_empty() {
        // 顶级单本：目录取自身，文件取父目录
        let full = Path::new(root).join(&comic.path);
        if comic.kind == "folder" {
            Some(full.to_string_lossy().to_string())
        } else {
            full.parent().map(|p| p.to_string_lossy().to_string())
        }
    } else if let Some(p) = by_id.get(&comic.series_id) {
        bucket_of(by_id, p)
    } else {
        None
    }
}

fn with_conn<T>(
    bucket_dir: &str,
    f: impl FnOnce(&Connection) -> Result<T, String>,
) -> Result<T, String> {
    let mut guard = CONNS.lock().map_err(|_| "封面库锁竞争".to_string())?;
    let map = guard.get_or_insert_with(HashMap::new);
    if !map.contains_key(bucket_dir) {
        let conn = Connection::open(cover_db_path(bucket_dir))
            .map_err(|e| format!("打开封面库失败: {e}"))?;
        conn.busy_timeout(Duration::from_secs(5)).ok();
        // 目录派生数据统一存外部库（唯一来源）：封面 BLOB + 手动封面参数 + 页面索引 + 页面预览
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS covers (
                path       TEXT PRIMARY KEY,
                data       BLOB NOT NULL,
                mime       TEXT NOT NULL DEFAULT 'image/jpeg',
                width      INTEGER DEFAULT 0,
                height     INTEGER DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS configs (
                path TEXT NOT NULL,
                key  TEXT NOT NULL,
                value TEXT NOT NULL DEFAULT '',
                PRIMARY KEY (path, key)
            );
            CREATE TABLE IF NOT EXISTS page_indices (
                path       TEXT PRIMARY KEY,
                data       TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS previews (
                path       TEXT NOT NULL,
                page_index INTEGER NOT NULL,
                data       BLOB,
                mime       TEXT NOT NULL DEFAULT 'image/jpeg',
                width      INTEGER DEFAULT 0,
                height     INTEGER DEFAULT 0,
                PRIMARY KEY (path, page_index)
            );
            "#,
        )
        .map_err(|e| format!("初始化封面库失败: {e}"))?;
        map.insert(bucket_dir.to_string(), conn);
    }
    f(map.get(bucket_dir).expect("刚插入的连接必然存在"))
}

/// 保存某章节封面（key 为记录相对本体目录的路径，见 `rel_key_of`）
pub fn save_cover(
    bucket_dir: &str,
    key: &str,
    data: &[u8],
    mime: &str,
    width: i64,
    height: i64,
) -> Result<(), String> {
    with_conn(bucket_dir, |conn| {
        conn.execute(
            "INSERT INTO covers (path, data, mime, width, height, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, datetime('now')) \
             ON CONFLICT(path) DO UPDATE SET \
               data = excluded.data, mime = excluded.mime, \
               width = excluded.width, height = excluded.height, \
               created_at = datetime('now')",
            params![key, data, mime, width, height],
        )
        .map_err(|e| format!("写入封面库失败: {e}"))?;
        Ok(())
    })
}

/// 读取某章节封面，未命中返回 None
pub fn load_cover(
    bucket_dir: &str,
    key: &str,
) -> Result<Option<(Vec<u8>, String, i64, i64)>, String> {
    with_conn(bucket_dir, |conn| {
        let mut stmt = conn
            .prepare("SELECT data, mime, width, height FROM covers WHERE path = ?1")
            .map_err(|e| format!("查询封面库失败: {e}"))?;
        let mut rows = stmt
            .query_map(params![key], |r| {
                Ok((
                    r.get::<_, Vec<u8>>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })
            .map_err(|e| format!("查询封面库失败: {e}"))?;
        rows.next()
            .transpose()
            .map_err(|e| format!("读取封面库失败: {e}"))
    })
}

/// 删除某章节封面
pub fn delete_cover(bucket_dir: &str, key: &str) -> Result<(), String> {
    with_conn(bucket_dir, |conn| {
        conn.execute("DELETE FROM covers WHERE path = ?1", params![key])
            .map_err(|e| format!("删除封面库失败: {e}"))?;
        Ok(())
    })
}

// ══════════════════════════════════════════════════════════
//  configs：单漫画手动封面参数（与封面 BLOB 同库，随目录走）
// ══════════════════════════════════════════════════════════

/// 读取某漫画的参数（key-value），无则空 map
pub fn load_configs(bucket_dir: &str, key: &str) -> Result<HashMap<String, String>, String> {
    with_conn(bucket_dir, |conn| {
        let mut stmt = conn
            .prepare("SELECT key, value FROM configs WHERE path = ?1")
            .map_err(|e| format!("查询参数失败: {e}"))?;
        let rows = stmt
            .query_map(params![key], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| format!("查询参数失败: {e}"))?;
        let mut map = HashMap::new();
        for r in rows {
            let (k, v) = r.map_err(|e| format!("读取参数失败: {e}"))?;
            map.insert(k, v);
        }
        Ok(map)
    })
}

/// 批量保存某漫画的参数（UPSERT）
pub fn save_configs(
    bucket_dir: &str,
    key: &str,
    config: &HashMap<String, String>,
) -> Result<(), String> {
    with_conn(bucket_dir, |conn| {
        for (k, v) in config {
            conn.execute(
                "INSERT INTO configs (path, key, value) VALUES (?1, ?2, ?3)
                 ON CONFLICT(path, key) DO UPDATE SET value = excluded.value",
                params![key, k, v],
            )
            .map_err(|e| format!("保存参数失败: {e}"))?;
        }
        Ok(())
    })
}

// ══════════════════════════════════════════════════════════
//  page_indices：章节页面条带索引缓存
// ══════════════════════════════════════════════════════════

/// 读取章节页面条带索引，未命中返回 None
pub fn load_page_index(bucket_dir: &str, key: &str) -> Result<Option<String>, String> {
    with_conn(bucket_dir, |conn| {
        conn.query_row(
            "SELECT data FROM page_indices WHERE path = ?1",
            params![key],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| format!("读取页索引失败: {e}"))
    })
}

/// 保存章节页面条带索引
pub fn save_page_index(bucket_dir: &str, key: &str, data: &str) -> Result<(), String> {
    with_conn(bucket_dir, |conn| {
        conn.execute(
            "INSERT INTO page_indices (path, data, updated_at) VALUES (?1, ?2, datetime('now'))
             ON CONFLICT(path) DO UPDATE SET data = excluded.data, updated_at = datetime('now')",
            params![key, data],
        )
        .map_err(|e| format!("保存页索引失败: {e}"))?;
        Ok(())
    })
}

// ══════════════════════════════════════════════════════════
//  previews：页面预览缩略图缓存
// ══════════════════════════════════════════════════════════

/// 读取某页预览图，未命中返回 None
pub fn load_preview(
    bucket_dir: &str,
    key: &str,
    page_index: usize,
) -> Result<Option<(Vec<u8>, String)>, String> {
    with_conn(bucket_dir, |conn| {
        conn.query_row(
            "SELECT data, mime FROM previews WHERE path = ?1 AND page_index = ?2",
            params![key, page_index as i64],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| format!("读取预览失败: {e}"))
    })
}

/// 保存某页预览图（UPSERT）
pub fn save_preview(
    bucket_dir: &str,
    key: &str,
    page_index: usize,
    data: &[u8],
    mime: &str,
    width: i64,
    height: i64,
) -> Result<(), String> {
    with_conn(bucket_dir, |conn| {
        conn.execute(
            "INSERT INTO previews (path, page_index, data, mime, width, height)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(path, page_index) DO UPDATE SET
               data = excluded.data, mime = excluded.mime,
               width = excluded.width, height = excluded.height",
            params![key, page_index as i64, data, mime, width, height],
        )
        .map_err(|e| format!("保存预览失败: {e}"))?;
        Ok(())
    })
}

/// 清理某本体库中已不在有效集合中的条目（covers / page_indices / previews）。
/// configs（手动封面参数）不在清理范围：它是用户数据，宁可残留也不误删——
/// 一旦 key 基准变化导致暂时失配，参数仍保留，不会造成手动封面配置丢失。
pub fn purge_missing(bucket_dir: &str, valid_paths: &[String]) -> Result<(), String> {
    with_conn(bucket_dir, |conn| {
        let valid: HashSet<&str> = valid_paths.iter().map(|s| s.as_str()).collect();
        // 各表都按 path 键组织，统一清理不在有效集合中的 path（configs 除外）
        for table in ["covers", "page_indices", "previews"] {
            let sql = format!("SELECT DISTINCT path FROM {table}");
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| format!("查询封面库失败: {e}"))?;
            let paths = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(|e| format!("查询封面库失败: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("读取封面库失败: {e}"))?;
            let stale: Vec<String> = paths
                .iter()
                .filter(|p| !valid.contains(p.as_str()))
                .cloned()
                .collect();
            for p in stale {
                conn.execute(&format!("DELETE FROM {table} WHERE path = ?1"), params![p])
                    .map_err(|e| format!("清理封面库失败: {e}"))?;
            }
        }
        Ok(())
    })
}

/// 扫描完成后按本体分组清理残留条目，并使归属缓存失效
pub fn purge_after_scan(comics: &[Comic]) {
    let by_id: HashMap<String, Comic> =
        comics.iter().map(|c| (c.id.clone(), c.clone())).collect();
    let mut buckets: HashMap<String, Vec<String>> = HashMap::new();
    for c in comics {
        if let Some(b) = bucket_of(&by_id, c) {
            // 有效集合用新基准：记录相对本体目录的 key
            buckets.entry(b.clone()).or_default().push(rel_key_of(c, &b));
        }
    }
    for (b, valid) in buckets {
        let _ = purge_missing(&b, &valid);
    }
    invalidate_buckets();
}

/// 清空全部本体外部封面库的可再生成数据（封面 / 页索引 / 预览），
/// 保留 configs（手动封面参数），下次"设为封面"时重新生成。
pub fn clear_all() {
    let dirs: HashSet<String> = buckets().into_values().collect();
    for d in dirs {
        let _ = with_conn(&d, |conn| {
            conn.execute("DELETE FROM covers", [])
                .map_err(|e| format!("清空封面库失败: {e}"))?;
            conn.execute("DELETE FROM page_indices", [])
                .map_err(|e| format!("清空页索引失败: {e}"))?;
            conn.execute("DELETE FROM previews", [])
                .map_err(|e| format!("清空预览失败: {e}"))?;
            Ok(())
        });
    }
}
