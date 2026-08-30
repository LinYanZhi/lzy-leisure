//! 休闲时光数据库层（单库分表）
//!
//! 单一数据库文件（app_data_dir/leisure.db），全局连接（Mutex 保护）。
//! 表结构沿用旧版（漫画书架 + 视频管理）的成熟设计，统一管理：
//!   - meta：全局设置（schema_version、ffmpeg 路径等）
//!   - comics / root_dirs：漫画书架
//!   - actors / tag_groups / tags / videos / video_actors / video_tags：视频管理
//!   - novels：小说（预留，后续功能接入）
//!
//! 数据归属规范（v3 起）：
//!   - 应用库只存"软件使用状态"：书架索引 / 阅读进度 / 排序 / 完结缓存
//!   - 目录派生数据（封面 BLOB / 手动封面参数 / 页面索引 / 页面预览）统一放
//!     各漫画本体目录的外部封面库（covers_db::.leisure-covers.db），随目录走。
//!
//! 子模块：
//!   - comics：漫画相关数据模型与操作
//!   - videos：视频/演员/标签相关数据模型与操作
#![allow(dead_code)]
pub mod comics;
pub mod novels;
pub mod storyclub;
pub mod videos;

use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

static DB: OnceLock<Mutex<Connection>> = OnceLock::new();

/// 随机视频 id 计数器（与时间戳拼接，保证进程内唯一）
static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// 当前 schema 版本（新增表/列时递增，并补对应迁移逻辑）
const SCHEMA_VERSION: i64 = 4;

// ══════════════════════════════════════════════════════════
//  初始化
// ══════════════════════════════════════════════════════════

/// 初始化数据库（建表 + 迁移）
pub fn init_db(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建数据目录失败: {e}"))?;
    }
    let conn = Connection::open(path).map_err(|e| format!("打开数据库失败: {e}"))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| format!("设置 busy_timeout 失败: {e}"))?;
    // WAL 只是性能优化，不是功能必需：若因文件被瞬时占用 / 杀软拦截 / 目录受限
    // 等导致设置失败，降级为默认 journal 模式继续，避免整个应用变"数据库未初始化"。
    if let Err(e) = conn.execute_batch("PRAGMA journal_mode=WAL;") {
        log::warn!("WAL 模式设置失败，降级为默认模式（不影响功能）: {e}");
        let _ = conn.execute_batch("PRAGMA journal_mode=DELETE;");
    }
    conn.execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(|e| format!("数据库 PRAGMA 设置失败: {e}"))?;

    migrate(&conn)?;
    let _ = DB.set(Mutex::new(conn)).map_err(|_| "数据库重复初始化".to_string());
    // v2 → v3：把暂存的手动封面参数写入各外部封面库。
    // 旧库积累的配置可能达数百上千条，同步执行会阻塞启动 → 放后台线程；
    // 失败时保留 pending 标记，下次启动自动重试。
    std::thread::spawn(|| {
        if let Err(e) = migrate_pending_cover_configs() {
            log::error!("手动封面参数迁移失败（下次启动将重试）: {e}");
        }
        // 清除旧封面缓存表后压缩一次数据库，释放文件空洞（后台执行，不阻塞启动）
        if let Err(e) = compact_db_if_needed() {
            log::warn!("数据库压缩失败（不影响功能）: {e}");
        }
    });
    log::info!("休闲时光数据库已初始化: {}", path.display());
    Ok(())
}

fn migrate(conn: &Connection) -> Result<(), String> {
    // v2 → v3：旧 comic_config（手动封面参数）暂存到 meta，待 DB 就绪后迁往外部封面库
    stash_cover_configs(conn)?;
    conn.execute_batch(
        r#"
        -- v2 → v3：目录派生数据（手动封面参数 / 页面索引 / 页面预览）已迁往
        -- 各漫画本体目录的外部封面库（.leisure-covers.db），应用库不再保留
        DROP TABLE IF EXISTS comic_config;
        DROP TABLE IF EXISTS page_indices;
        DROP TABLE IF EXISTS comic_previews;
        -- 更早版本残留的旧封面缓存表（封面已迁外部库，此处一并清除）
        DROP TABLE IF EXISTS comic_covers;

        -- ── 全局设置（共享） ──
        CREATE TABLE IF NOT EXISTS meta (
            key   TEXT PRIMARY KEY,
            value TEXT
        );

        -- ── 漫画书架 ──
        CREATE TABLE IF NOT EXISTS comics (
            id               TEXT PRIMARY KEY,
            title            TEXT NOT NULL,
            path             TEXT NOT NULL,
            type             TEXT NOT NULL,
            format           TEXT NOT NULL,
            page_count       INTEGER DEFAULT 0,
            size             INTEGER DEFAULT 0,
            root_dir         TEXT NOT NULL,
            series_id        TEXT DEFAULT '',
            cover_path       TEXT DEFAULT '',
            reading_progress INTEGER DEFAULT 0,
            chapter_count    INTEGER DEFAULT 0,
            sort_order       INTEGER DEFAULT 0,
            is_container     INTEGER NOT NULL DEFAULT 0,
            completed        INTEGER,
            updated_at       TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS root_dirs (
            path       TEXT PRIMARY KEY,
            name       TEXT NOT NULL,
            sort_order INTEGER DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_comics_root ON comics(root_dir);
        CREATE INDEX IF NOT EXISTS idx_comics_series ON comics(series_id);

        -- ── 视频管理 ──
        CREATE TABLE IF NOT EXISTS actors (
            id          TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            stage_names TEXT NOT NULL DEFAULT '[]',
            height      TEXT NOT NULL DEFAULT '',
            cup_size    TEXT NOT NULL DEFAULT '',
            birthdate   TEXT NOT NULL DEFAULT '',
            bio         TEXT NOT NULL DEFAULT '',
            images      TEXT NOT NULL DEFAULT '[]',
            sort_order  INTEGER NOT NULL DEFAULT 0,
            created_at  TEXT NOT NULL DEFAULT '',
            updated_at  TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS tag_groups (
            id         TEXT PRIMARY KEY,
            name       TEXT NOT NULL,
            sort_order INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS tags (
            id         TEXT PRIMARY KEY,
            name       TEXT NOT NULL,
            color      TEXT NOT NULL DEFAULT '#8a8a8a',
            group_id   TEXT NOT NULL DEFAULT '',
            sort_order INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS videos (
            id            TEXT PRIMARY KEY,
            title         TEXT NOT NULL,
            path          TEXT NOT NULL UNIQUE,
            description   TEXT NOT NULL DEFAULT '',
            license_plate TEXT NOT NULL DEFAULT '',
            cover         TEXT NOT NULL DEFAULT '',
            duration      TEXT NOT NULL DEFAULT '',
            file_size     TEXT NOT NULL DEFAULT '',
            file_type     TEXT NOT NULL DEFAULT '',
            fps           REAL,
            frame_width   INTEGER,
            frame_height  INTEGER,
            sort_order    INTEGER NOT NULL DEFAULT 0,
            created_at    TEXT NOT NULL DEFAULT '',
            updated_at    TEXT NOT NULL DEFAULT '',
            kinds         TEXT NOT NULL DEFAULT '[]',
            subtitle_path TEXT NOT NULL DEFAULT '',
            root_dir       TEXT NOT NULL DEFAULT '',
            series_id      TEXT NOT NULL DEFAULT '',
            progress       REAL NOT NULL DEFAULT 0,
            year           TEXT NOT NULL DEFAULT '',
            rating         REAL NOT NULL DEFAULT 0,
            episode        TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS video_actors (
            video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
            actor_id TEXT NOT NULL REFERENCES actors(id) ON DELETE CASCADE,
            PRIMARY KEY (video_id, actor_id)
        );
        CREATE TABLE IF NOT EXISTS video_tags (
            video_id TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
            tag_id   TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
            PRIMARY KEY (video_id, tag_id)
        );
        CREATE INDEX IF NOT EXISTS idx_videos_sort   ON videos(sort_order, created_at);
        CREATE INDEX IF NOT EXISTS idx_tags_group    ON tags(group_id);
        CREATE INDEX IF NOT EXISTS idx_va_actor      ON video_actors(actor_id);
        CREATE INDEX IF NOT EXISTS idx_vt_tag        ON video_tags(tag_id);

        -- ── 视频导入路径（源） ──
        CREATE TABLE IF NOT EXISTS video_roots (
            path       TEXT PRIMARY KEY,
            name       TEXT NOT NULL,
            sort_order INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT ''
        );

        -- ── 视频剧集（Series）：多个视频聚合为一部剧/一个合集 ──
        CREATE TABLE IF NOT EXISTS series (
            id          TEXT PRIMARY KEY,
            title       TEXT NOT NULL,
            cover       TEXT NOT NULL DEFAULT '',
            description TEXT NOT NULL DEFAULT '',
            sort_order  INTEGER NOT NULL DEFAULT 0,
            created_at  TEXT NOT NULL DEFAULT '',
            updated_at  TEXT NOT NULL DEFAULT ''
        );

        -- ── 小说（预留） ──
        CREATE TABLE IF NOT EXISTS novels (
            id          TEXT PRIMARY KEY,
            title       TEXT NOT NULL,
            author      TEXT NOT NULL DEFAULT '',
            path        TEXT NOT NULL DEFAULT '',
            cover_path  TEXT NOT NULL DEFAULT '',
            reading_pos INTEGER DEFAULT 0,
            chapter     TEXT NOT NULL DEFAULT '',
            sort_order  INTEGER DEFAULT 0,
            created_at  TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );

        -- ── 故事会（单一路径：根目录下每个子目录 = 一年，内含 PDF 期数） ──
        -- fingerprint：内容指纹（文件大小 + 前 64KB 哈希），文件改名/移动后不变，
        -- 重扫时据此归并原记录，阅读进度等软数据不丢
        CREATE TABLE IF NOT EXISTS storyclub_issues (
            id               TEXT PRIMARY KEY,
            title            TEXT NOT NULL,
            year             TEXT NOT NULL DEFAULT '',
            path             TEXT NOT NULL UNIQUE,
            page_count       INTEGER DEFAULT 0,
            size             INTEGER DEFAULT 0,
            reading_progress INTEGER DEFAULT 0,
            sort_order       INTEGER DEFAULT 0,
            fingerprint      TEXT NOT NULL DEFAULT '',
            updated_at       TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_storyclub_year ON storyclub_issues(year);
        "#,
    )
    .map_err(|e| format!("建表失败: {e}"))?;

    // ── is_container 列迁移（旧库补列，幂等） ──
    // 语义：1=集合父级（role=collection 目录生成的卡片，不作为书架单元展示）
    let has_container: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('comics') WHERE name = 'is_container'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if !has_container {
        conn.execute_batch("ALTER TABLE comics ADD COLUMN is_container INTEGER NOT NULL DEFAULT 0;")
            .map_err(|e| format!("迁移 is_container 列失败: {e}"))?;
        // 旧数据尽力推断（仅升级首启执行一次）：含 series 类型子项（子项是独立漫画而非章节）
        // 的 series 记录 → 集合父级。重扫时由扫描器精确重写（UPSERT 更新本列），
        // 后续启动不再推断，避免误标「系列套系列」这类结构。
        conn.execute_batch(
            "UPDATE comics SET is_container = 1
             WHERE is_container = 0 AND type = 'series'
               AND EXISTS (SELECT 1 FROM comics c2 WHERE c2.series_id = comics.id AND c2.type = 'series');",
        )
        .map_err(|e| format!("迁移 is_container 推断失败: {e}"))?;
    }

    // ── completed 列迁移（v3 补列，幂等） ──
    // index.json 仍为完结标志的权威来源，本列仅为书架加载的 DB 缓存，
    // 首次查询时惰性回填（见 commands::get_completed_status）。
    let has_completed: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('comics') WHERE name = 'completed'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if !has_completed {
        conn.execute_batch("ALTER TABLE comics ADD COLUMN completed INTEGER;")
            .map_err(|e| format!("迁移 completed 列失败: {e}"))?;
    }

    // ── videos.kinds 列迁移（v4 补列，幂等） ──
    // 种类（电影/短视频/动漫/竖屏，可多选）存 JSON 数组字符串，如 '["movie","anime"]'
    let has_kinds: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('videos') WHERE name = 'kinds'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if !has_kinds {
        conn.execute_batch("ALTER TABLE videos ADD COLUMN kinds TEXT NOT NULL DEFAULT '[]';")
            .map_err(|e| format!("迁移 videos.kinds 列失败: {e}"))?;
    }

    // ── videos.subtitle_path 列迁移（v5 补列，幂等） ──
    // 同目录完全同名（仅扩展名不同）的 ass/srt/ssa/vtt 字幕文件路径，扫描时登记
    let has_subtitle: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('videos') WHERE name = 'subtitle_path'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if !has_subtitle {
        conn.execute_batch(
            "ALTER TABLE videos ADD COLUMN subtitle_path TEXT NOT NULL DEFAULT '';",
        )
        .map_err(|e| format!("迁移 videos.subtitle_path 列失败: {e}"))?;
    }

    // ── videos.root_dir 列迁移（幂等） ──
    // 视频归属的导入路径（源）；扫描时填充，用于按源过滤与整源移除
    let has_root_dir: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('videos') WHERE name = 'root_dir'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if !has_root_dir {
        conn.execute_batch("ALTER TABLE videos ADD COLUMN root_dir TEXT NOT NULL DEFAULT '';")
            .map_err(|e| format!("迁移 videos.root_dir 列失败: {e}"))?;
    }
    // 索引需在列补全之后创建：旧库建表批次被跳过时 root_dir 列尚不存在，
    // 提前建索引会导致 migrate 失败（数据库未初始化）。
    conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_videos_root ON videos(root_dir);")
        .map_err(|e| format!("创建 videos.root_dir 索引失败: {e}"))?;

    // ── videos.series_id 列迁移（幂等） ──
    // 剧集归属：空串表示不属于任何剧集；删除剧集时统一置空
    let has_series_id: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('videos') WHERE name = 'series_id'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if !has_series_id {
        conn.execute_batch("ALTER TABLE videos ADD COLUMN series_id TEXT NOT NULL DEFAULT '';")
            .map_err(|e| format!("迁移 videos.series_id 列失败: {e}"))?;
    }
    conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_videos_series ON videos(series_id);")
        .map_err(|e| format!("创建 videos.series_id 索引失败: {e}"))?;

    // ── videos.progress 列迁移（幂等） ──
    // 播放进度（秒），用于续播；0 表示未观看
    let has_progress: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('videos') WHERE name = 'progress'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if !has_progress {
        conn.execute_batch("ALTER TABLE videos ADD COLUMN progress REAL NOT NULL DEFAULT 0;")
            .map_err(|e| format!("迁移 videos.progress 列失败: {e}"))?;
    }

    // ── videos.year / rating / episode 列迁移（幂等，全部追加在末尾保持列序一致） ──
    for (col, ddl) in [
        (
            "year",
            "ALTER TABLE videos ADD COLUMN year TEXT NOT NULL DEFAULT '';",
        ),
        (
            "rating",
            "ALTER TABLE videos ADD COLUMN rating REAL NOT NULL DEFAULT 0;",
        ),
        (
            "episode",
            "ALTER TABLE videos ADD COLUMN episode TEXT NOT NULL DEFAULT '';",
        ),
    ] {
        let has_col: bool = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM pragma_table_info('videos') WHERE name = '{col}'"),
                [],
                |r| r.get(0),
            )
            .unwrap_or(false);
        if !has_col {
            conn.execute_batch(&ddl)
                .map_err(|e| format!("迁移 videos.{col} 列失败: {e}"))?;
        }
    }

    // ── 视频 id 稳定化迁移（一次）：id 从 md5(路径) 改为随机 id ──
    // 否则文件改名/移动后 id 变化，演员/标签/剧集/进度等关联全部丢失。
    // 关联表（video_actors / video_tags）随 id 同步更新；
    // series.cover_video_id 为动态子查询（list_series/get_series 时计算），无需迁移。
    let id_v2_done: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM meta WHERE key = 'video_id_v2_done'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;
    if !id_v2_done {
        let old_ids: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT id FROM videos")
                .map_err(|e| format!("读取视频 id 失败: {e}"))?;
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(|e| format!("读取视频 id 失败: {e}"))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("读取视频 id 失败: {e}"))?
        };
        if !old_ids.is_empty() {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("开启迁移事务失败: {e}"))?;
            for old in &old_ids {
                let new = new_video_id();
                tx.execute(
                    "UPDATE video_actors SET video_id = ?1 WHERE video_id = ?2",
                    params![new, old],
                )
                .map_err(|e| format!("迁移视频演员关联失败: {e}"))?;
                tx.execute(
                    "UPDATE video_tags SET video_id = ?1 WHERE video_id = ?2",
                    params![new, old],
                )
                .map_err(|e| format!("迁移视频标签关联失败: {e}"))?;
                tx.execute(
                    "UPDATE videos SET id = ?1 WHERE id = ?2",
                    params![new, old],
                )
                .map_err(|e| format!("迁移视频 id 失败: {e}"))?;
            }
            tx.commit().map_err(|e| format!("提交迁移事务失败: {e}"))?;
            log::info!("视频 id 稳定化迁移完成（{} 条）", old_ids.len());
        }
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('video_id_v2_done', '1')
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [],
        )
        .map_err(|e| format!("写入视频 id 迁移标记失败: {e}"))?;
    }

    // ── 数据自愈：回填 videos.root_dir（幂等，仅处理空值） ──
    // 早期版本入库的视频 root_dir 为空（旧代码无归属概念 / upsert 漏写该列），
    // 导致源管理中按 `v.root_dir = r.path` 统计的视频数量为 0。
    // 对已登记的导入路径，将其下视频（path 以 root 为前缀）归属回填。
    conn.execute_batch(
        r#"
        UPDATE videos SET root_dir = (
            SELECT r.path FROM video_roots r
            WHERE videos.path = r.path
               OR substr(videos.path, 1, length(r.path) + 1) = r.path || '\'
               OR substr(videos.path, 1, length(r.path) + 1) = r.path || '/'
            LIMIT 1
        ) WHERE root_dir = '';
        UPDATE video_roots SET created_at = datetime('now', 'localtime') WHERE created_at = '';
        "#,
    )
    .map_err(|e| format!("回填视频归属路径失败: {e}"))?;

    // ── storyclub_issues.fingerprint 列迁移（幂等） ──
    // 内容指纹（文件大小 + 前 64KB 哈希）：文件改名/移动后内容不变、指纹不变，
    // 重扫时据此归并原记录（replace_root_issues 按指纹匹配），阅读进度等软数据不丢。
    // 与 videos 的「id 随机化」同理，但故事会期数数量大、需保持整表替换的秒级扫描，
    // 故不动 id（仍 md5(路径)），仅新增指纹列作为归并键。
    let has_fp: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('storyclub_issues') WHERE name = 'fingerprint'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);
    if !has_fp {
        conn.execute_batch(
            "ALTER TABLE storyclub_issues ADD COLUMN fingerprint TEXT NOT NULL DEFAULT '';",
        )
        .map_err(|e| format!("迁移 storyclub_issues.fingerprint 列失败: {e}"))?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_storyclub_fingerprint ON storyclub_issues(fingerprint);",
    )
    .map_err(|e| format!("创建 storyclub_issues.fingerprint 索引失败: {e}"))?;

    // ── novels 表列迁移（幂等）：root_dir（归属源）/ chapter_count（章节数缓存）/ chapters_json（章节列表缓存） ──
    // 小说模块：单本 EPUB 即一部书；章节列表为解析缓存（可重扫刷新），不随文件路径外置。
    for (col, ddl) in [
        (
            "root_dir",
            "ALTER TABLE novels ADD COLUMN root_dir TEXT NOT NULL DEFAULT '';",
        ),
        (
            "chapter_count",
            "ALTER TABLE novels ADD COLUMN chapter_count INTEGER NOT NULL DEFAULT 0;",
        ),
        (
            "chapters_json",
            "ALTER TABLE novels ADD COLUMN chapters_json TEXT NOT NULL DEFAULT '[]';",
        ),
    ] {
        let has_col: bool = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM pragma_table_info('novels') WHERE name = '{col}'"),
                [],
                |r| r.get(0),
            )
            .unwrap_or(false);
        if !has_col {
            conn.execute_batch(&ddl)
                .map_err(|e| format!("迁移 novels.{col} 列失败: {e}"))?;
        }
    }

    // ── 小说导入路径（源） ──
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS novel_roots (
            path       TEXT PRIMARY KEY,
            name       TEXT NOT NULL,
            sort_order INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT ''
        );
        "#,
    )
    .map_err(|e| format!("建 novel_roots 表失败: {e}"))?;

    // 记录 schema 版本
    conn.execute(
        "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![SCHEMA_VERSION.to_string()],
    )
    .map_err(|e| format!("写入 schema_version 失败: {e}"))?;
    Ok(())
}

/// v2 → v3：旧 comic_config（手动封面参数）暂存到 meta。
/// 仅在旧库（comic_config 表存在且有数据）执行一次，随后表被 DROP。
fn stash_cover_configs(conn: &Connection) -> Result<(), String> {
    let has_table: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'comic_config'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
        > 0;
    if !has_table {
        return Ok(());
    }
    let rows: Vec<(String, String, String)> = {
        let mut stmt = conn
            .prepare("SELECT comic_id, key, value FROM comic_config")
            .map_err(|e| format!("读取旧配置失败: {e}"))?;
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
            })
            .map_err(|e| format!("读取旧配置失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("读取旧配置失败: {e}"))?;
        drop(stmt);
        rows
    };
    if rows.is_empty() {
        return Ok(());
    }
    let json = serde_json::to_string(&rows).map_err(|e| format!("序列化旧配置失败: {e}"))?;
    conn.execute(
        "INSERT INTO meta (key, value) VALUES ('pending_cover_configs', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![json],
    )
    .map_err(|e| format!("暂存旧配置失败: {e}"))?;
    log::info!("已暂存 {} 条手动封面参数待迁移", rows.len());
    Ok(())
}

/// v2 → v3 遗留：把暂存的手动封面参数写入各外部封面库 configs（随目录走）。
/// 需在 DB 全局连接就绪后调用（内部依赖 comics 表与归属计算）。
/// 在后台线程执行：旧库积累的配置可能很多，同步执行会阻塞启动。
fn migrate_pending_cover_configs() -> Result<(), String> {
    let pending = get_setting("pending_cover_configs")?.unwrap_or_default();
    if pending.is_empty() {
        return Ok(());
    }
    let rows: Vec<(String, String, String)> =
        serde_json::from_str(&pending).map_err(|e| format!("解析暂存配置失败: {e}"))?;
    let all = comics::load_all_comics()?;
    let by_id: HashMap<String, &crate::db::comics::Comic> =
        all.iter().map(|c| (c.id.clone(), c)).collect();
    // 归属映射只计算一次；按 (bucket, path) 分组批量写入，避免逐条开外部库连接
    let buckets = crate::covers_db::all_buckets();
    let mut per_key: HashMap<(String, String), HashMap<String, String>> = HashMap::new();
    for (comic_id, key, value) in &rows {
        if let Some(comic) = by_id.get(comic_id) {
            if let Some(bucket) = buckets.get(comic_id) {
                let cover_key = crate::covers_db::rel_key_of(comic, bucket);
                per_key
                    .entry((bucket.clone(), cover_key))
                    .or_default()
                    .insert(key.clone(), value.clone());
            }
        }
    }
    for ((bucket, path), m) in per_key {
        let _ = crate::covers_db::save_configs(&bucket, &path, &m);
    }
    set_setting("pending_cover_configs", "")?;
    log::info!("手动封面参数迁移完成（{} 条）", rows.len());
    Ok(())
}

/// 数据库压缩（一次性）：旧版 comic_covers 表被 DROP 后文件空洞不释放，
/// 首次启动在后台 VACUUM 一次瘦身，完成后写标记不再执行。
fn compact_db_if_needed() -> Result<(), String> {
    if get_setting("db_compact_done")?.is_some() {
        return Ok(());
    }
    let conn = lock()?;
    conn.execute_batch("VACUUM;")
        .map_err(|e| format!("压缩数据库失败: {e}"))?;
    drop(conn);
    set_setting("db_compact_done", "1")?;
    log::info!("数据库压缩完成");
    Ok(())
}

fn lock() -> Result<MutexGuard<'static, Connection>, String> {
    DB.get()
        .ok_or_else(|| "数据库未初始化".to_string())?
        .lock()
        .map_err(|e| format!("数据库锁获取失败: {e}"))
}

// ══════════════════════════════════════════════════════════
//  公共工具
// ══════════════════════════════════════════════════════════

/// 当前时间（本地时间，ISO 格式）。
/// 注意：内部会获取数据库锁，调用方必须先于 lock() 调用，避免 Mutex 非重入死锁。
pub fn now() -> String {
    lock()
        .ok()
        .and_then(|conn| {
            conn.query_row("SELECT datetime('now', 'localtime')", [], |row| row.get(0))
                .ok()
        })
        .unwrap_or_default()
}

/// 漫画/章节稳定 ID：root+rel 的 md5 前 12 位（与旧版一致）
pub fn generate_id(root: &str, rel_path: &str) -> String {
    let digest = md5::compute(format!("{root}{rel_path}"));
    format!("{:x}", digest)[..12].to_string()
}

/// 视频/演员/标签稳定 ID：输入路径或名称的 md5 前 16 位
pub fn video_id(input: &str) -> String {
    let digest = format!("{:x}", md5::compute(input.as_bytes()));
    digest[..16].to_string()
}

/// 新视频的稳定 ID：随机生成（不依赖路径，不获取数据库锁）。
/// 文件改名/移动后 id 不变，演员/标签/剧集/进度等关联不丢；
/// 扫描以 path 为匹配键（get_video_by_path），同路径幂等。
pub fn new_video_id() -> String {
    let n = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seed = format!("{ts}:{n}");
    let digest = format!("{:x}", md5::compute(seed.as_bytes()));
    digest[..16].to_string()
}

/// 读取全局设置
pub fn get_setting(key: &str) -> Result<Option<String>, String> {
    let conn = lock()?;
    conn.query_row(
        "SELECT value FROM meta WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .map(|v| Some(v))
    .or_else(|e| {
        if e == rusqlite::Error::QueryReturnedNoRows {
            Ok(None)
        } else {
            Err(format!("查询设置失败: {e}"))
        }
    })
}

/// 保存全局设置
pub fn set_setting(key: &str, value: &str) -> Result<(), String> {
    let conn = lock()?;
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map_err(|e| format!("保存设置失败: {e}"))?;
    Ok(())
}

// ══════════════════════════════════════════════════════════
//  工具
// ══════════════════════════════════════════════════════════

/// 自然排序 key：数字部分按数值比较，其余按小写字符串。
/// 用 bytes 切片分段（零拷贝），仅对 ASCII 折叠大小写——
/// 中文无大小写，避免 Unicode to_lowercase 与逐字符收集（10 万级文件名的排序性能关键）。
pub fn natural_key(text: &str) -> Vec<NaturalPart> {
    let mut parts = Vec::new();
    if text.is_empty() {
        return parts;
    }
    let bytes = text.as_bytes();
    let mut start = 0usize;
    let mut is_digit = bytes[0].is_ascii_digit();
    for (i, &b) in bytes.iter().enumerate().skip(1) {
        let d = b.is_ascii_digit();
        if d != is_digit {
            push_segment(&mut parts, &text[start..i], is_digit);
            start = i;
            is_digit = d;
        }
    }
    push_segment(&mut parts, &text[start..], is_digit);
    parts
}

#[inline]
fn push_segment(parts: &mut Vec<NaturalPart>, seg: &str, is_digit: bool) {
    if is_digit {
        parts.push(NaturalPart::Num(seg.parse::<i64>().unwrap_or(0)));
    } else {
        parts.push(NaturalPart::Str(seg.to_ascii_lowercase()));
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum NaturalPart {
    Num(i64),
    Str(String),
}
