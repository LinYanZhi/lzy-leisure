// 验证：重扫后演员-视频关联、番号、AV 种类
import { DatabaseSync } from 'node:sqlite';
const db = new DatabaseSync(process.argv[2], { readOnly: true });
const links = db.prepare("SELECT COUNT(*) AS c FROM video_actors").get().c;
console.log('video_actors 关联数: ' + links);
const plates = db.prepare("SELECT COUNT(*) AS c FROM videos WHERE license_plate != ''").get().c;
console.log('已提取番号的视频: ' + plates);
const av = db.prepare("SELECT COUNT(*) AS c FROM videos WHERE kinds LIKE '%\"av\"%'").get().c;
console.log('标为 AV 种类的视频: ' + av);
console.log('\n=== 演员作品数 ===');
const rows = db.prepare(`
  SELECT a.name, COUNT(va.video_id) AS n FROM actors a
  LEFT JOIN video_actors va ON va.actor_id = a.id
  GROUP BY a.id ORDER BY n DESC
`).all();
for (const r of rows) console.log((r.name + '   ').slice(0, 10) + r.n + ' 部');
db.close();
