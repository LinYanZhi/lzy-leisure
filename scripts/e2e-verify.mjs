// 端到端验证：模拟 app 的完整交互链路
const API = 'http://127.0.0.1:5184/api';
const PASSWORD = process.argv[2];
const headers = { 'Content-Type': 'application/json', 'X-Auth': PASSWORD };

async function post(cmd, payload) {
  const res = await fetch(`${API}/${cmd}`, { method: 'POST', headers, body: JSON.stringify(payload) });
  const j = await res.json();
  if (!j.ok) throw new Error(`${cmd}: ${j.error}`);
  return j.data;
}

// 1. 演员列表 + 作品数
const actors = await post('list_actors_with_counts', {});
console.log(`演员总数: ${actors.length}`);
const withCount = actors.filter((a) => a.video_count > 0);
console.log(`有作品的演员: ${withCount.length} 位`);

// 2. 枫可怜链路
const fk = actors.find((a) => a.name === '枫可怜');
console.log(`枫可怜: ${fk.video_count} 部影片`);
const vids = await post('list_videos', { query: { actor_ids: [fk.id] } });
console.log(`按演员查询: ${vids.length} 部, 样例番号: ${vids.slice(0, 4).map((v) => v.license_plate || '-').join(', ')}`);
const img = await post('get_actor_image_data_url', { actor_id: fk.id });
console.log(`枫可怜头像 dataURL: ${img.length} 字符, 前缀: ${img.slice(0, 30)}`);

// 3. AV 分类
const av = await post('list_videos', { query: { kinds: ['av'] } });
console.log(`AV 分类影片: ${av.length} 部`);

// 4. 演员作品数 Top5
const top = [...actors].sort((a, b) => b.video_count - a.video_count).slice(0, 5);
console.log(`\n作品数 Top5: ${top.map((a) => `${a.name}(${a.video_count})`).join(' ')}`);
