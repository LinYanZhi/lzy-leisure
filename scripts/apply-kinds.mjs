// 按用户确认的分类批量打标（走 app HTTP API，保留其他字段）
import { DatabaseSync } from 'node:sqlite';

const DB = process.argv[2];
const API = 'http://127.0.0.1:5184/api';
const PASSWORD = process.argv[3];

const db = new DatabaseSync(DB, { readOnly: true });

// 目标：cat2 → av（movie 误标 + 未分类）；other → anime（未分类）
const cat2Av = db.prepare(
  "SELECT id FROM videos WHERE root_dir=? AND (kinds LIKE '%movie%' OR kinds = '[]')"
).all('D:\\视频\\cat-catch\\cat2');
const otherAnime = db.prepare(
  "SELECT id FROM videos WHERE root_dir=? AND kinds = '[]'"
).all('D:\\视频\\cat-catch\\other');
db.close();

console.log(`cat2 → av: ${cat2Av.length} 个 | other → anime: ${otherAnime.length} 个`);

const headers = { 'Content-Type': 'application/json', 'X-Auth': PASSWORD };
async function post(cmd, payload) {
  const res = await fetch(`${API}/${cmd}`, { method: 'POST', headers, body: JSON.stringify(payload) });
  const j = await res.json();
  if (!j.ok) throw new Error(`${cmd}: ${j.error}`);
  return j.data;
}

async function setKinds(id, kinds) {
  const v = await post('get_video', { video_id: id });
  if (!v) throw new Error(`get_video 为空: ${id}`);
  await post('update_video', {
    edit: {
      id: v.id,
      title: v.title,
      description: v.description,
      license_plate: v.license_plate,
      year: v.year,
      rating: v.rating,
      kinds,
      actor_ids: v.actors || [],
      tag_ids: v.tags || [],
    },
  });
}

let ok = 0, fail = 0;
for (const { id } of cat2Av) {
  try { await setKinds(id, ['av']); ok++; }
  catch (e) { console.log(`❌ ${id} av: ${e.message}`); fail++; }
}
for (const { id } of otherAnime) {
  try { await setKinds(id, ['anime']); ok++; }
  catch (e) { console.log(`❌ ${id} anime: ${e.message}`); fail++; }
}
console.log(`\n完成: 成功 ${ok}, 失败 ${fail}`);
