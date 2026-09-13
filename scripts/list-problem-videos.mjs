// 列出 cat2 movie 误标 + other 全部标题（只读）
import { DatabaseSync } from 'node:sqlite';
const db = new DatabaseSync(process.argv[2], { readOnly: true });

console.log('=== cat2 标为 movie 的视频 ===');
for (const v of db.prepare(
  "SELECT title, duration FROM videos WHERE root_dir=? AND kinds LIKE '%movie%' ORDER BY title"
).all('D:\\视频\\cat-catch\\cat2')) {
  console.log('  ' + v.title.slice(0, 42) + '  [' + v.duration + ']');
}

console.log('\n=== other 全部标题 ===');
for (const v of db.prepare(
  "SELECT title, duration, file_size FROM videos WHERE root_dir=? ORDER BY title"
).all('D:\\视频\\cat-catch\\other')) {
  console.log('  ' + v.title.slice(0, 46) + '  [' + v.duration + ']');
}
db.close();
