// 验证重命名后两个视频在 DB 中的路径
import { DatabaseSync } from 'node:sqlite';
const db = new DatabaseSync(process.argv[2], { readOnly: true });
const rows = db.prepare("SELECT title, path, license_plate FROM videos WHERE license_plate IN ('IPX-477','MIDV-592')").all();
for (const r of rows) console.log(`${r.license_plate} | ${r.title.slice(0, 40)} | ${r.path}`);
console.log(`总数: ${rows.length}`);
db.close();
