// 验证国产视频（MD 系列）在 DB 中的路径
import { DatabaseSync } from 'node:sqlite';
const db = new DatabaseSync(process.argv[2], { readOnly: true });
const rows = db.prepare("SELECT license_plate, path FROM videos WHERE license_plate LIKE 'MD-%'").all();
for (const r of rows) {
  const name = r.path.split(/[\\/]/).pop();
  console.log(`${r.license_plate} | ${name}`);
}
console.log(`国产视频数: ${rows.length}`);
db.close();
