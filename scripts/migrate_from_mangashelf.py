# -*- coding: utf-8 -*-
"""从旧项目 MangaShelf 的 mangashelf.db 迁移自定义封面到新项目封面库。

旧版数据结构：
  - comics.root_dir = 作品目录（如 D:\\漫画\\堕落物语），series 本体 path='.'，章节 path=相对作品目录路径
  - comic_covers(comic_id, quality='standard', data, mime, width, height)  标准封面 BLOB
  - comic_config(comic_id, key='cover_crop_offset' 等)                     手动封面裁剪参数

新项目封面库（.leisure-covers.db，位于漫画本体目录下）：
  - covers(path, data, mime, width, height)  key = 记录相对本体目录的路径（本体自身为空串）
  - configs(path, key, value)                key = 同上

映射：旧 path（相对作品目录）直接就是新 key（'.' 视为本体自身 → 空串）。
作品目录可能已从 D:\\漫画\\<作品> 移到 D:\\漫画\\《神奇漫画》\\<作品>，
按作品名在新项目库中匹配实际本体目录。

覆盖策略（目标 key 已存在时）：
  - 新库 configs 已有该 key 的手动参数（用户在新项目手动设置过）→ 保留新库
  - 否则 → 用旧自定义封面覆盖（旧数据 > 自动生成）
只迁移 quality='standard' 的封面与 cover_crop_offset 参数。
"""
import os
import shutil
import sqlite3
import glob
import sys

OLD_DBS = [
    r"c:\Users\LinYanZhi\Code\MangaShelf\mangashelf.db",
    r"c:\Users\LinYanZhi\Code\MangaShelf\mangashelf.db.bak",
    r"c:\Users\LinYanZhi\Code\MangaShelf\dist\MangaShelf\mangashelf.db",
]
MANGA_ROOT = r"D:\漫画"
BACKUP_DIR = r"c:\Users\LinYanZhi\AppData\Local\Temp\covers_backup_ms"

# 新项目库目录索引：作品名 -> [本体目录...]（按与旧 root 路径相似度排序由调用方处理）
NEW_LIB_DIRS = [os.path.dirname(d) for d in glob.glob(os.path.join(MANGA_ROOT, "**", ".leisure-covers.db"), recursive=True)]
NAME_INDEX = {}
for d in NEW_LIB_DIRS:
    NAME_INDEX.setdefault(os.path.basename(d), []).append(d)

SCHEMA = """
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
"""

MANUAL_KEYS = ("cover_crop_offset", "cover_crop_y", "cover_pdf_page", "cover_pdf_offset")


def pick_bucket(old_root: str):
    """返回旧 root 对应的新本体目录；无法确定返回 None。"""
    if os.path.isdir(old_root):
        return old_root
    name = os.path.basename(old_root)
    cands = NAME_INDEX.get(name, [])
    if not cands:
        return None
    # 选与 old_root 公共前缀最长的（多个同名目录时最贴近原位置）
    norm = lambda p: p.replace("\\", "/").lower()
    best = max(cands, key=lambda c: _common_prefix_len(norm(old_root), norm(c)))
    return best


def _common_prefix_len(a: str, b: str):
    n = 0
    for x, y in zip(a, b):
        if x != y:
            break
        n += 1
    return n


def load_old_data(db: str):
    """读取单个旧库，返回 {comic_id: (root_dir, path, cover, configs)}。"""
    if not os.path.exists(db):
        return {}
    conn = sqlite3.connect("file:" + db.replace("\\", "/") + "?mode=ro", uri=True)
    conn.text_factory = lambda b: b.decode("utf-8", "replace")
    cur = conn.cursor()
    comics = {}
    try:
        for r in cur.execute("SELECT id, root_dir, path FROM comics"):
            comics[r[0]] = {"root_dir": r[1], "path": r[2], "cover": None, "configs": {}}
        for r in cur.execute("SELECT comic_id, data, mime, width, height FROM comic_covers WHERE quality='standard' AND data IS NOT NULL AND length(data)>0"):
            cid, data, mime, w, h = r
            if cid in comics:
                comics[cid]["cover"] = (data, mime or "image/webp", w or 0, h or 0)
        for r in cur.execute("SELECT comic_id, key, value FROM comic_config WHERE key IN ('cover_crop_offset','cover_crop_y')"):
            cid, k, v = r
            if cid in comics:
                comics[cid]["configs"][k] = v
    except sqlite3.OperationalError:
        return {}
    conn.close()
    return comics


def ensure_conn(bucket: str) -> sqlite3.Connection:
    conn = sqlite3.connect(os.path.join(bucket, ".leisure-covers.db"), timeout=10)
    conn.execute("PRAGMA busy_timeout=8000")
    conn.text_factory = lambda b: b.decode("utf-8", "replace")
    conn.executescript(SCHEMA)
    return conn


def main():
    os.makedirs(BACKUP_DIR, exist_ok=True)
    # 合并旧数据（按库顺序，先到先得：主库优先）
    merged = {}
    order = []
    for db in OLD_DBS:
        data = load_old_data(db)
        for cid, rec in data.items():
            if cid not in merged:
                merged[cid] = rec
                order.append(cid)
    print(f"旧库合并记录: {len(merged)} 条")

    # 解析 (bucket, key)，有封面或参数才处理
    targets = {}  # (bucket, key) -> (cover, configs)
    no_match = []
    for cid in order:
        rec = merged[cid]
        if rec["cover"] is None and not rec["configs"]:
            continue
        bucket = pick_bucket(rec["root_dir"])
        if bucket is None:
            no_match.append((rec["root_dir"], rec["path"]))
            continue
        key = "" if rec["path"] in (".", "") else rec["path"].replace("\\", "/")
        if (bucket, key) not in targets:  # 主库优先
            targets[(bucket, key)] = (rec["cover"], rec["configs"])

    print(f"待写入目标 (bucket,key): {len(targets)}，未匹配作品目录: {len(no_match)}")
    for r in no_match[:20]:
        print("   [未匹配]", r)

    migrated = skipped = 0
    conns = {}
    backups = set()
    for (bucket, key), (cover, configs) in sorted(targets.items()):
        lib = os.path.join(bucket, ".leisure-covers.db")
        if bucket not in backups:
            if os.path.exists(lib):
                try:
                    shutil.copy2(lib, os.path.join(BACKUP_DIR, f"{os.path.basename(bucket)}.leisure-covers.db"))
                except OSError:
                    pass
            backups.add(bucket)
        conn = conns.get(bucket)
        if conn is None:
            conn = ensure_conn(bucket)
            conns[bucket] = conn
        cur = conn.cursor()
        # 检查新库现状
        row = cur.execute("SELECT 1 FROM covers WHERE path=?", (key,)).fetchone()
        has_cover = row is not None
        manual = cur.execute(
            "SELECT 1 FROM configs WHERE path=? AND key IN ('cover_crop_offset','cover_pdf_page')",
            (key,),
        ).fetchone()
        if has_cover and manual:
            skipped += 1
            continue
        if cover is not None:
            data, mime, w, h = cover
            cur.execute(
                "INSERT INTO covers (path, data, mime, width, height, created_at) VALUES (?,?,?,?,?, datetime('now')) "
                "ON CONFLICT(path) DO UPDATE SET data=excluded.data, mime=excluded.mime, width=excluded.width, height=excluded.height, created_at=datetime('now')",
                (key, data, mime, w, h),
            )
        for k, v in configs.items():
            if k in ("cover_crop_offset", "cover_crop_y"):
                cur.execute(
                    "INSERT INTO configs (path, key, value) VALUES (?,?,?) ON CONFLICT(path,key) DO UPDATE SET value=excluded.value",
                    (key, k, v),
                )
        migrated += 1
    for conn in conns.values():
        conn.commit()
        conn.close()

    print(f"\n迁移 {migrated} 条，跳过（新库已有手动封面）{skipped} 条")
    print(f"备份目录: {BACKUP_DIR}")


if __name__ == "__main__":
    sys.exit(main())
