// 上传 saopinyi 官方头像到 app（替换原视频封面头像）
import fs from 'node:fs';
import path from 'node:path';

const API = 'http://127.0.0.1:5184/api';
const PASSWORD = process.argv[2];
const AVATAR_DIR = process.argv[3];

const ACTORS = {
  '明里紬': '94f971db9d34d7c4', '三好佑香': '91875da04303b775', '新有菜': '43b9a95d038c745f',
  '枫可怜': '0e09c858f7f61ba0', '天使萌': '6a15113d8dc39034', '三上悠亚': '00d0ef5d52a8de0e',
  '桃乃木香奈': '0c4bb033d9f6fb68', '河北彩花': 'aada3e4955a35d26', '凪ひかる': 'ea72637ed8d3f49f',
  '滨崎真绪': '1ce5407df63de88b', '本庄铃': '0e959cf577c2d832', '森泽佳奈': 'f609015cf777b170',
  '樱空桃': '11cce2e7ad722668', '楪可怜': '62bbbc90a2888be8', '枫芙爱': '422745f8d3815b43',
  '山岸逢花': 'f219c35df4280e9c', '向井蓝': '17d1a846b1491466', '七濑爱丽丝': '8fe8ff26ae25a5c4',
  '仲村美优': 'eb16bd183f4983ff', '鹫尾芽衣': '690a485de97e26ae', '新井优香': '95262422ef080be2',
};

let ok = 0, fail = 0;
for (const [name, id] of Object.entries(ACTORS)) {
  const file = path.join(AVATAR_DIR, `${name}.jpg`);
  if (!fs.existsSync(file)) { console.log(`❌ 缺少文件: ${name}`); fail++; continue; }
  const bytes = fs.readFileSync(file);
  const dataUrl = `data:image/jpeg;base64,${bytes.toString('base64')}`;
  try {
    const res = await fetch(`${API}/upload_actor_image`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'X-Auth': PASSWORD },
      body: JSON.stringify({ actor_id: id, data_url_value: dataUrl }),
    });
    const j = await res.json();
    if (j.ok) { console.log(`✅ ${name} (${bytes.length}B)`); ok++; }
    else { console.log(`⚠️ ${name}: ${j.error}`); fail++; }
  } catch (e) { console.log(`❌ ${name}: ${e.message}`); fail++; }
}
console.log(`\n完成: 成功 ${ok}, 失败 ${fail}`);
