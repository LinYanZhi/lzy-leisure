// 为全部演员填充头像：
// 1) 优先用 cat-catch 根目录的演员命名图片；
// 2) 否则用该演员名下某部视频的封面（.leisure-video-covers.db）；
// 3) 通过 app 的 HTTP API upload_actor_image 上传（X-Auth 口令）。
import { DatabaseSync } from 'node:sqlite';
import fs from 'node:fs';
import path from 'node:path';

const LEISURE_DB = process.argv[2];
const PASSWORD = process.argv[3];
const CAT_CATCH = 'D:\\视频\\cat-catch';
const API = 'http://127.0.0.1:5184/api';

const db = new DatabaseSync(LEISURE_DB, { readOnly: true });
const actors = db.prepare("SELECT id, name, stage_names, images FROM actors ORDER BY sort_order, created_at").all();
const videos = db.prepare("SELECT path FROM videos").all();
db.close();

const norm = (s) => (s || '').toLowerCase().replace(/[·・、,， _　]/g, '');

// ── 来源 1：cat-catch 根目录的演员命名图片 ──
const localImages = fs.readdirSync(CAT_CATCH)
  .filter((f) => /\.(jpg|jpeg|png|webp)$/i.test(f))
  .map((f) => path.join(CAT_CATCH, f));

function localImageFor(actor) {
  const names = [actor.name, ...JSON.parse(actor.stage_names || '[]')]
    .filter((n) => n && n.length >= 2);
  for (const img of localImages) {
    const base = norm(path.basename(img, path.extname(img)));
    for (const n of names) {
      const nn = norm(n);
      if (nn.length >= 2 && base.includes(nn)) return img;
    }
  }
  return null;
}

// ── 来源 2：演员名下某部视频的封面 ──
function coverFor(actor) {
  const names = [actor.name, ...JSON.parse(actor.stage_names || '[]')]
    .filter((n) => n && n.length >= 2);
  const video = videos.find((v) => {
    const lp = v.path.toLowerCase();
    return names.some((n) => n.length >= 2 && lp.includes(n.toLowerCase()));
  });
  if (!video) return null;
  const coverDb = path.join(path.dirname(video.path), '.leisure-video-covers.db');
  if (!fs.existsSync(coverDb)) return null;
  try {
    const cdb = new DatabaseSync(coverDb, { readOnly: true });
    const row = cdb.prepare('SELECT data, mime FROM covers WHERE path=?').get(path.basename(video.path));
    cdb.close();
    return row ? { bytes: row.data, mime: row.mime } : null;
  } catch {
    return null;
  }
}

// ── 上传 ──
async function uploadAvatar(actorId, bytes, mime) {
  const dataUrl = `data:${mime};base64,${Buffer.from(bytes).toString('base64')}`;
  const res = await fetch(`${API}/upload_actor_image`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', 'X-Auth': PASSWORD },
    body: JSON.stringify({ actor_id: actorId, data_url_value: dataUrl }),
  });
  const j = await res.json();
  return j.ok === true;
}

// ── 执行 ──
let ok = 0, local = 0, cover = 0, missing = 0;
for (const a of actors) {
  if (JSON.parse(a.images || '[]').length > 0) {
    console.log(`跳过（已有头像）: ${a.name}`);
    ok++;
    continue;
  }
  const localImg = localImageFor(a);
  let src = null, label = '';
  if (localImg) {
    const ext = path.extname(localImg).toLowerCase();
    const mime = ext === '.png' ? 'image/png' : ext === '.webp' ? 'image/webp' : 'image/jpeg';
    src = { bytes: fs.readFileSync(localImg), mime };
    label = `本地图片 ${path.basename(localImg)}`;
    local++;
  } else {
    const c = coverFor(a);
    if (c) {
      src = { bytes: Buffer.from(c.bytes), mime: c.mime };
      label = `视频封面`;
      cover++;
    }
  }
  if (!src) {
    console.log(`❌ 无来源: ${a.name}`);
    missing++;
    continue;
  }
  try {
    const up = await uploadAvatar(a.id, src.bytes, src.mime);
    console.log(`${up ? '✅' : '⚠️'} ${a.name} ← ${label}${up ? '' : ' 上传失败'}`);
    if (up) ok++;
  } catch (e) {
    console.log(`❌ ${a.name} 上传异常: ${e.message}`);
    missing++;
  }
}
console.log(`\n完成：成功 ${ok}（本地 ${local} / 封面 ${cover}），失败/缺源 ${missing}`);
