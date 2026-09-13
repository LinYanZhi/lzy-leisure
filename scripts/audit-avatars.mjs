// 盘点演员头像现状 + 每个演员的可用于提取封面的视频（只读）
import { DatabaseSync } from 'node:sqlite';
const dbPath = process.argv[2];
const db = new DatabaseSync(dbPath, { readOnly: true });
try {
  const actors = db.prepare("SELECT id, name, stage_names, images FROM actors ORDER BY sort_order, created_at").all();
  console.log('=== 演员头像现状 ===');
  for (const a of actors) {
    const imgs = JSON.parse(a.images || '[]');
    const cnt = db.prepare("SELECT COUNT(*) AS c FROM video_actors WHERE actor_id=?").get(a.id).c;
    console.log(`${a.id}\t${a.name}\t头像:${imgs.length > 0 ? '有' : '无'}\t作品:${cnt}`);
  }
  console.log('\n=== 每演员第一部视频（用于封面提取） ===');
  for (const a of actors) {
    const v = db.prepare(`
      SELECT v.path, v.title FROM videos v
      JOIN video_actors va ON va.video_id = v.id
      WHERE va.actor_id = ? ORDER BY v.sort_order, v.created_at LIMIT 1
    `).get(a.id);
    if (v) console.log(`${a.id}\t${a.name}\t${v.path}`);
  }
} finally {
  db.close();
}
