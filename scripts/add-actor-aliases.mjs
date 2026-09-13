// 为演员补充缺失的艺名写法（明里䌷 等），通过 HTTP API save_actor 更新
const API = 'http://127.0.0.1:5184/api';
const PASSWORD = process.argv[2];

const headers = { 'Content-Type': 'application/json', 'X-Auth': PASSWORD };

async function post(cmd, payload) {
  const res = await fetch(`${API}/${cmd}`, {
    method: 'POST',
    headers,
    body: JSON.stringify(payload),
  });
  return res.json();
}

// 补充映射：演员名 → 需要追加的别名（只补确定出现的写法）
const EXTRA = {
  '明里紬': ['明里䌷'],
};

const list = await post('list_actors', {});
const actors = list.data || [];
let updated = 0;
for (const a of actors) {
  const extra = EXTRA[a.name];
  if (!extra) continue;
  const existing = new Set(a.stage_names || []);
  const added = extra.filter((x) => !existing.has(x));
  if (added.length === 0) continue;
  const input = {
    id: a.id,
    name: a.name,
    stage_names: [...(a.stage_names || []), ...added],
    height: a.height || '',
    cup_size: a.cup_size || '',
    birthdate: a.birthdate || '',
    bio: a.bio || '',
    sort_order: a.sort_order || 0,
  };
  const r = await post('save_actor', { input });
  console.log(`${a.name}: 追加别名 [${added.join(', ')}] → ${r.ok ? 'OK' : 'FAIL ' + r.error}`);
  updated++;
}
console.log(`共更新 ${updated} 位演员`);
