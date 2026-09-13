// 分析视频源与种类现状（只读）
import { DatabaseSync } from 'node:sqlite';
const db = new DatabaseSync(process.argv[2], { readOnly: true });

console.log('=== 视频源（roots） ===');
const roots = db.prepare("SELECT path, name FROM video_roots ORDER BY sort_order, created_at").all();
for (const r of roots) {
  const n = db.prepare("SELECT COUNT(*) AS c FROM videos WHERE root_dir = ?").get(r.path).c;
  console.log(`${r.path}  (${n} 个视频)`);
}

console.log('\n=== 各源种类分布 ===');
for (const r of roots) {
  const rows = db.prepare("SELECT kinds FROM videos WHERE root_dir = ?").all(r.path);
  const dist = {};
  for (const row of rows) {
    const kinds = JSON.parse(row.kinds || '[]');
    const key = kinds.length === 0 ? '(未分类)' : kinds.join('+');
    dist[key] = (dist[key] || 0) + 1;
  }
  console.log(`\n[${r.name || r.path}]`);
  for (const [k, v] of Object.entries(dist).sort((a, b) => b[1] - a[1])) console.log(`  ${k}: ${v}`);
}

console.log('\n=== 疑似误标：带车牌(番号)却标了 movie ===');
const mislabeled = db.prepare(
  "SELECT title, license_plate, kinds, root_dir FROM videos WHERE license_plate != '' AND kinds LIKE '%\"movie\"%' ORDER BY root_dir"
).all();
for (const v of mislabeled) console.log(`  [${v.license_plate}] ${v.title.slice(0, 40)} | kinds=${v.kinds} | ${v.root_dir}`);

console.log('\n=== 未分类视频（kinds 空）抽样 ===');
const unclassified = db.prepare(
  "SELECT title, license_plate, root_dir, file_type FROM videos WHERE kinds = '[]' ORDER BY root_dir LIMIT 40"
).all();
for (const v of unclassified) console.log(`  [${v.license_plate || '无车牌'}] ${v.title.slice(0, 45)} | ${v.root_dir}`);

console.log('\n=== 各源带车牌/无车牌统计 ===');
for (const r of roots) {
  const all = db.prepare("SELECT COUNT(*) AS c FROM videos WHERE root_dir = ?").get(r.path).c;
  const plate = db.prepare("SELECT COUNT(*) AS c FROM videos WHERE root_dir = ? AND license_plate != ''").get(r.path).c;
  console.log(`  ${r.name || r.path}: ${all} 个，带车牌 ${plate}`);
}
