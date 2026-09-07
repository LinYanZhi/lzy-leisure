// ============================================================
// 休闲时光 - 目录 / 文件选择（原 api.ts 拆分）
// 桌面用系统对话框（Tauri dialog），浏览器用后端目录浏览弹窗。
// ============================================================
import { open } from "@tauri-apps/plugin-dialog";
import { authHeaders, handleUnauthorized, isWeb } from "./transport";

interface FsEntry {
  name: string;
  path: string;
}

interface FsList {
  path: string;
  parent: string | null;
  dirs: FsEntry[];
}

/** 浏览器模式：调用后端目录浏览接口 */
async function fsList(path: string): Promise<FsList> {
  const res = await fetch(`/api/fs/list?path=${encodeURIComponent(path)}`, {
    headers: authHeaders(),
  });
  if (res.status === 401) {
    handleUnauthorized();
  }
  const json = await res.json().catch(() => null);
  if (!json || json.ok !== true) {
    throw new Error(json?.error || `目录读取失败 (HTTP ${res.status})`);
  }
  return json.data as FsList;
}

/**
 * 目录选择弹窗（原生 DOM，浏览器模式用；桌面模式走 Tauri dialog）。
 * 返回选中目录绝对路径；取消返回 null。
 */
function openDirPicker(title: string): Promise<string | null> {
  return new Promise((resolve) => {
    const overlay = document.createElement("div");
    overlay.style.cssText =
      "position:fixed;inset:0;background:rgba(0,0,0,.55);z-index:9999;display:flex;align-items:center;justify-content:center;";
    const box = document.createElement("div");
    box.style.cssText =
      "background:#1e1e1e;color:#eee;width:min(620px,92vw);max-height:74vh;border-radius:8px;display:flex;flex-direction:column;overflow:hidden;box-shadow:0 8px 32px rgba(0,0,0,.5);";
    const header = document.createElement("div");
    header.style.cssText =
      "padding:12px 16px;font-weight:600;border-bottom:1px solid #333;display:flex;justify-content:space-between;align-items:center;";
    const titleEl = document.createElement("span");
    titleEl.textContent = title;
    const closeBtn = document.createElement("button");
    closeBtn.textContent = "✕";
    closeBtn.style.cssText =
      "background:none;border:none;color:#aaa;font-size:16px;cursor:pointer;line-height:1;";
    header.append(titleEl, closeBtn);

    const pathBar = document.createElement("div");
    pathBar.style.cssText =
      "padding:8px 16px;font-size:12px;color:#aaa;word-break:break-all;border-bottom:1px solid #2a2a2a;min-height:34px;box-sizing:border-box;";

    const list = document.createElement("div");
    list.style.cssText = "flex:1;overflow-y:auto;padding:8px;";

    const footer = document.createElement("div");
    footer.style.cssText =
      "padding:10px 16px;border-top:1px solid #333;display:flex;justify-content:flex-end;gap:8px;";
    const cancelBtn = document.createElement("button");
    cancelBtn.textContent = "取消";
    cancelBtn.className = "btn";
    const okBtn = document.createElement("button");
    okBtn.textContent = "选择此目录";
    okBtn.className = "btn btn-primary";
    okBtn.style.minWidth = "110px";
    footer.append(cancelBtn, okBtn);

    box.append(header, pathBar, list, footer);
    overlay.append(box);
    document.body.append(overlay);

    let currentPath = "";
    let okEnabled = false;

    function close(v: string | null) {
      document.body.removeChild(overlay);
      resolve(v);
    }

    function renderEntry(name: string, onClick: () => void) {
      const row = document.createElement("div");
      row.textContent = name;
      row.style.cssText =
        "padding:8px 12px;border-radius:6px;cursor:pointer;font-size:14px;display:flex;align-items:center;gap:8px;";
      row.addEventListener("mouseenter", () => (row.style.background = "#2a2a2a"));
      row.addEventListener("mouseleave", () => (row.style.background = "transparent"));
      row.addEventListener("click", onClick);
      return row;
    }

    function load(path: string) {
      fsList(path)
        .then((res) => {
          currentPath = res.path;
          pathBar.textContent = res.path || "（选择盘符）";
          list.innerHTML = "";
          if (res.parent) {
            list.append(renderEntry("⬆ 上级目录", () => load(res.parent!)));
          }
          if (res.dirs.length === 0) {
            const empty = document.createElement("div");
            empty.textContent = "（无子目录）";
            empty.style.cssText = "padding:16px;color:#666;font-size:13px;text-align:center;";
            list.append(empty);
          }
          for (const d of res.dirs) {
            const target = d.path;
            const row = renderEntry("📁 " + d.name, () => load(target));
            row.title = d.path;
            list.append(row);
          }
          okEnabled = res.path !== "";
        })
        .catch((e) => {
          pathBar.textContent = String(e);
        });
    }

    closeBtn.addEventListener("click", () => close(null));
    cancelBtn.addEventListener("click", () => close(null));
    okBtn.addEventListener("click", () => {
      if (okEnabled) close(currentPath);
    });
    overlay.addEventListener("click", (e) => {
      if (e.target === overlay) close(null);
    });

    load("");
  });
}

/** 选择目录：桌面用系统对话框，浏览器用后端目录浏览弹窗 */
export async function pickDirectory(title: string): Promise<string | null> {
  if (!isWeb) {
    const result = await open({ directory: true, multiple: false, title });
    return typeof result === "string" ? result : null;
  }
  return openDirPicker(title);
}

/** 选择文件（仅浏览器模式：列出当前目录下的视频文件供选择） */
export function pickFile(title: string): Promise<string | null> {
  if (!isWeb) {
    return open({ directory: false, multiple: false, title }).then((r) =>
      typeof r === "string" ? r : null,
    );
  }
  return openFilePicker(title, VIDEO_EXTS, "🎬");
}

/** 选择小说文件（EPUB / TXT）：桌面走系统对话框带过滤器，浏览器列出 epub/txt */
export function pickNovelFile(title: string): Promise<string | null> {
  if (!isWeb) {
    return open({
      directory: false,
      multiple: false,
      title,
      filters: [
        { name: "小说", extensions: ["epub", "txt"] },
        { name: "所有文件", extensions: ["*"] },
      ],
    }).then((r) => (typeof r === "string" ? r : null));
  }
  return openFilePicker(title, NOVEL_EXTS, "📖");
}

const VIDEO_EXTS = new Set([
  "mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v", "ts", "rmvb", "mpg", "mpeg",
]);

const NOVEL_EXTS = new Set(["epub", "txt"]);

/** 浏览器模式：文件选择弹窗（列出指定扩展名文件） */
function openFilePicker(
  title: string,
  exts: Set<string>,
  emoji: string,
): Promise<string | null> {
  return new Promise((resolve) => {
    const overlay = document.createElement("div");
    overlay.style.cssText =
      "position:fixed;inset:0;background:rgba(0,0,0,.55);z-index:9999;display:flex;align-items:center;justify-content:center;";
    const box = document.createElement("div");
    box.style.cssText =
      "background:#1e1e1e;color:#eee;width:min(620px,92vw);max-height:74vh;border-radius:8px;display:flex;flex-direction:column;overflow:hidden;box-shadow:0 8px 32px rgba(0,0,0,.5);";
    const header = document.createElement("div");
    header.style.cssText =
      "padding:12px 16px;font-weight:600;border-bottom:1px solid #333;display:flex;justify-content:space-between;align-items:center;";
    const titleEl = document.createElement("span");
    titleEl.textContent = title;
    const closeBtn = document.createElement("button");
    closeBtn.textContent = "✕";
    closeBtn.style.cssText =
      "background:none;border:none;color:#aaa;font-size:16px;cursor:pointer;line-height:1;";
    header.append(titleEl, closeBtn);

    const pathBar = document.createElement("div");
    pathBar.style.cssText =
      "padding:8px 16px;font-size:12px;color:#aaa;word-break:break-all;border-bottom:1px solid #2a2a2a;min-height:34px;box-sizing:border-box;";

    const list = document.createElement("div");
    list.style.cssText = "flex:1;overflow-y:auto;padding:8px;";

    const footer = document.createElement("div");
    footer.style.cssText =
      "padding:10px 16px;border-top:1px solid #333;display:flex;justify-content:flex-end;gap:8px;";
    const cancelBtn = document.createElement("button");
    cancelBtn.textContent = "取消";
    cancelBtn.className = "btn";
    footer.append(cancelBtn);
    box.append(header, pathBar, list, footer);
    overlay.append(box);
    document.body.append(overlay);

    function close(v: string | null) {
      document.body.removeChild(overlay);
      resolve(v);
    }

    function load(path: string) {
      fsList(path)
        .then((res) => {
          pathBar.textContent = res.path || "（选择盘符）";
          list.innerHTML = "";
          if (res.parent) {
            const up = document.createElement("div");
            up.textContent = "⬆ 上级目录";
            up.style.cssText =
              "padding:8px 12px;border-radius:6px;cursor:pointer;font-size:14px;color:#8ab4f8;";
            up.addEventListener("click", () => load(res.parent!));
            list.append(up);
          }
          if (res.dirs.length === 0) {
            const empty = document.createElement("div");
            empty.textContent = "（当前目录无文件）";
            empty.style.cssText = "padding:16px;color:#666;font-size:13px;text-align:center;";
            list.append(empty);
          }
          for (const d of res.dirs) {
            const target = d.path;
            const row = document.createElement("div");
            row.textContent = "📁 " + d.name;
            row.style.cssText =
              "padding:8px 12px;border-radius:6px;cursor:pointer;font-size:14px;";
            row.addEventListener("mouseenter", () => (row.style.background = "#2a2a2a"));
            row.addEventListener("mouseleave", () => (row.style.background = "transparent"));
            row.addEventListener("click", () => load(target));
            row.title = d.path;
            list.append(row);
          }
          // 列出当前目录下的视频文件（由 fs_list 提供文件列表）
          const files = (res as unknown as { files?: FsEntry[] }).files || [];
          for (const f of files) {
            const ext = f.name.split(".").pop()?.toLowerCase() || "";
            if (!exts.has(ext)) continue;
            const row = document.createElement("div");
            row.textContent = `${emoji} ` + f.name;
            row.style.cssText =
              "padding:8px 12px;border-radius:6px;cursor:pointer;font-size:14px;color:#7ee787;";
            row.addEventListener("mouseenter", () => (row.style.background = "#2a2a2a"));
            row.addEventListener("mouseleave", () => (row.style.background = "transparent"));
            row.addEventListener("click", () => close(f.path));
            row.title = f.path;
            list.append(row);
          }
        })
        .catch((e) => {
          pathBar.textContent = String(e);
        });
    }

    closeBtn.addEventListener("click", () => close(null));
    cancelBtn.addEventListener("click", () => close(null));
    overlay.addEventListener("click", (e) => {
      if (e.target === overlay) close(null);
    });

    load("");
  });
}
