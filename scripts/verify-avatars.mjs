// 验证演员头像已写入 DB
import { DatabaseSync } from 'node:sqlite';
const db = new DatabaseSync(process.argv[2], { readOnly: true });
const rows = db.prepare("SELECT name, images FROM actors ORDER BY sort_order").all();
let withImg = 0;
for (const r of rows) {
  const imgs = JSON.parse(r.images || '[]');
  if (imgs.length > 0) withImg++;
  console.log((r.name + '   ').slice(0, 10) + (imgs.length > 0 ? '有头像: ' + imgs[0] : '无'));
}
console.log('有头像总数: ' + withImg + '/' + rows.length);
db.close();
