// 读取 leisure.db 的 web_password（只读）
import { DatabaseSync } from 'node:sqlite';
const dbPath = process.argv[2];
const db = new DatabaseSync(dbPath, { readOnly: true });
try {
  const row = db.prepare("SELECT value FROM meta WHERE key='web_password'").get();
  console.log('web_password=' + (row ? row.value : 'NONE'));
  // 顺带确认 actors 表结构与条数
  const n = db.prepare("SELECT COUNT(*) AS c FROM actors").get();
  console.log('actors_count=' + n.c);
  const roots = db.prepare("SELECT path FROM video_roots").all();
  console.log('video_roots=' + JSON.stringify(roots.map(r => r.path)));
} finally {
  db.close();
}
