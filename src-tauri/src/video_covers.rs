//! 视频目录级封面库（封面跟随视频文件走）
//!
//! 在视频文件所在目录下维护一个 `.leisure-video-covers.db`，统一保存该目录下
//! 各视频的封面 BLOB。key 为视频文件名（相对其父目录），目录内唯一。
//! 封面跟着视频目录走：整体拷贝某个视频目录到别处后，重新导入即可直接复用，
//! 用户手动设置/上传的封面不丢失。应用库（leisure.db）只存视频源/路径/软元数据，
//! 不存封面。
//!
//! 与漫画外部封面库（covers_db）同一设计思路，仅表收敛为 covers：
//! 视频封面是纯缓存、可再生成，因此所有操作失败一律由调用方降级
//! （读取视为未命中、写入静默忽略），绝不阻塞主流程。
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

/// 视频封面库文件名（位于视频文件所在目录下；非视频扩展名，扫描器自动忽略）
pub const VIDEO_COVER_DB_FILE: &str = ".leisure-video-covers.db";

/// 父目录 → 已打开的外部库连接（SQLite 连接非 Sync，全局锁串行化访问）
static CONNS: Mutex<Option<HashMap<String, Connection>>> = Mutex::new(None);

/// 视频封面库完整路径（视频父目录下）
pub fn cover_db_path(bucket_dir: &str) -> std::path::PathBuf {
    Path::new(bucket_dir).join(VIDEO_COVER_DB_FILE)
}

/// 视频所属封面库目录（父目录）
pub fn bucket_dir(video_path: &str) -> Option<String> {
    Path::new(video_path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
}

/// 封面库 key：视频文件名（相对父目录）
pub fn rel_key_of(video_path: &str) -> Option<String> {
    Path::new(video_path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
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
            "#,
        )
        .map_err(|e| format!("初始化封面库失败: {e}"))?;
        map.insert(bucket_dir.to_string(), conn);
    }
    f(map.get(bucket_dir).expect("刚插入的连接必然存在"))
}

/// 保存视频封面（UPSERT）
pub fn save_cover(
    video_path: &str,
    data: &[u8],
    mime: &str,
    width: i64,
    height: i64,
) -> Result<(), String> {
    let bucket = bucket_dir(video_path).ok_or_else(|| "视频路径无效".to_string())?;
    let key = rel_key_of(video_path).ok_or_else(|| "视频路径无效".to_string())?;
    with_conn(&bucket, |conn| {
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

/// 读取视频封面，未命中（或目录不可写/不存在）返回 None
pub fn load_cover(video_path: &str) -> Option<(Vec<u8>, String)> {
    let bucket = bucket_dir(video_path)?;
    let key = rel_key_of(video_path)?;
    with_conn(&bucket, |conn| {
        conn.query_row(
            "SELECT data, mime FROM covers WHERE path = ?1",
            params![key],
            |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|e| format!("读取封面库失败: {e}"))
    })
    .ok()
    .flatten()
}
