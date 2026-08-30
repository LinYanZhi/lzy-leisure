# -*- coding: utf-8 -*-
"""把 .leisure-covers.db 中封面 key 从"相对根目录路径"迁移为"相对本体目录路径"。

背景：旧版 key = comics.path（相对根目录），根目录层级一变就失配；
新版 key = 记录相对其本体目录（库文件所在目录）的路径，只随本体目录结构变化。

用法：改 ROOT 为实际漫画根目录后运行 `python migrate_cover_keys.py`。
算法：对库内每个旧 key，枚举 bucket 的所有祖先目录作为候选"根"，
若 key 恰为 bucket 相对该根的路径 → 新 key 为空串（本体自身）；
若 key 以该前缀 + 分隔符开头 → 新 key = 截断前缀后的剩余路径。
取第一个匹配（从最近的祖先开始）。无法解释的 key 保留原样，绝不删除。
迁移前自动备份到 BACKUP_DIR。
"""
import os
import sqlite3
import shutil
import sys

ROOT = r"D:\漫画"
BACKUP_DIR = r"c:\Users\LinYanZhi\AppData\Local\Temp\covers_backup"


def ancestors(bucket: str):
    """bucket 的所有祖先目录（不含自身），从最近到最远。"""
    cur = bucket
    while True:
        parent = os.path.dirname(cur)
        if not parent or parent == cur:
            break
        yield parent
        cur = parent


def new_key_for(bucket: str, old: str):
    """尝试把旧 key 重算为相对本体目录的 key；无法解释返回 None。"""
    if not old:
        return None
    for anc in ancestors(bucket):
        p = os.path.relpath(bucket, anc).replace("\\", "/")
        if not p:
            continue
        if old == p:
            return ""  # 本体自身
        if old.startswith(p + "/") or old.startswith(p + "\\"):
            return old[len(p) + 1:]
    return None


def migrate_file(db: str):
    bucket = os.path.dirname(db)
    os.makedirs(BACKUP_DIR, exist_ok=True)
    bak = os.path.join(BACKUP_DIR, os.path.basename(db))
    try:
        shutil.copy2(db, bak)
    except OSError as e:
        print(f"  [跳过] 备份失败: {e}")
        return None

    conn = sqlite3.connect(db, timeout=10)
    conn.text_factory = lambda b: b.decode("utf-8", "replace")
    cur = conn.cursor()
    moved = 0
    kept = 0
    for table, cols, sel in (
        ("covers", "path, data, mime, width, height, created_at",
         "data, mime, width, height, created_at"),
        ("configs", "path, key, value",
         "key, value"),
    ):
        try:
            cur.execute(f"SELECT DISTINCT path FROM {table}")
            old_keys = [r[0] for r in cur.fetchall()]
        except sqlite3.OperationalError:
            continue  # 表不存在
        for old in old_keys:
            new = new_key_for(bucket, old)
            if new is None or new == old:
                kept += 1
                continue
            # INSERT OR REPLACE 迁到新 key（冲突时保留后迁入的），再删旧行
            cur.execute(
                f"INSERT OR REPLACE INTO {table} ({cols}) "
                f"SELECT ?1, {sel} FROM {table} WHERE path = ?2",
                (new, old),
            )
            cur.execute(f"DELETE FROM {table} WHERE path = ?1", (old,))
            moved += 1
    conn.commit()
    conn.close()
    return moved, kept


def main():
    dbs = []
    for base, dirs, files in os.walk(ROOT):
        dirs[:] = [d for d in dirs if not d.startswith(".") and not d.startswith("_")]
        for f in files:
            if f == ".leisure-covers.db":
                dbs.append(os.path.join(base, f))
    print(f"发现 {len(dbs)} 个封面库")
    total_moved = 0
    for db in sorted(dbs):
        r = migrate_file(db)
        if r is None:
            print(f"### {db}\n    [备份失败，未迁移]")
            continue
        moved, kept = r
        total_moved += moved
        print(f"### {db}\n    迁移 {moved} 条，保留 {kept} 条")
    print(f"\n共迁移 {total_moved} 条。备份目录: {BACKUP_DIR}")


if __name__ == "__main__":
    sys.exit(main())
