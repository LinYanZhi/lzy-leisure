// 故事会 PDF 加载与封面渲染共享工具（阅读器与书架后台封面补齐共用）
import { GlobalWorkerOptions, getDocument, type PDFDocumentProxy } from "pdfjs-dist";
// legacy 版 worker 内置 Uint8Array.prototype.toHex 等较新 API 的 polyfill，
// 旧版手机浏览器（Chromium <132 / Safari <18.4 无原生 toHex）也能正常解析 PDF。
import pdfWorkerUrl from "pdfjs-dist/legacy/build/pdf.worker.min.mjs?url";
import { convertFileSrc } from "@tauri-apps/api/core";
import { api, isWeb } from "../../api";

GlobalWorkerOptions.workerSrc = pdfWorkerUrl;

/**
 * 加载 PDF。
 * - 桌面窗口：asset 协议本地文件流式（毫秒级，免传输）。
 * - 浏览器/手机：直接全量 base64。原 URL Range 流在部分移动端内核
 *   （Trae 预览、旧 Chromium/Safari）会中断且不返回错误（net::ERR_ABORTED / 无限挂起），
 *   导致阅读器一直黑屏；局域网带宽足够，30MB 约数秒。
 */
export async function loadStoryPdf(issueId: string): Promise<PDFDocumentProxy> {
  if (!isWeb) {
    const url = convertFileSrc(await api.storyclubGetPdfPath(issueId));
    return await getDocument({ url }).promise;
  }
  const b64 = await api.storyclubGetPdfData(issueId);
  const bin = atob(b64);
  const data = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) data[i] = bin.charCodeAt(i);
  return await getDocument({ data }).promise;
}

/**
 * 后端直接提取第一页 JPEG（扫描件封面页多为整页 JPEG 图片，毫秒级，免去 pdf.js 解析整个大 PDF）。
 * 第一页无整页 JPEG（矢量/文字页）或 PDF 损坏时返回 null，由调用方退回 pdf.js 渲染。
 */
export async function tryFirstPageJpeg(issueId: string): Promise<string | null> {
  try {
    return await api.storyclubFirstPageJpeg(issueId);
  } catch {
    return null;
  }
}

/**
 * 渲染第一页为 jpeg data URL（封面来源）。
 * 按目标宽度等比缩放（默认 320px）：扫描件大图降采样后再编码，
 * 避免超大 canvas 的 JPEG 编码在主线程长时间卡顿（封面显示仅需几百 px 宽）。
 */
export async function renderFirstPageDataUrl(
  doc: PDFDocumentProxy,
  targetWidth = 320,
): Promise<string> {
  const page = await doc.getPage(1);
  const base = page.getViewport({ scale: 1 });
  const scale = targetWidth / base.width;
  const vp = page.getViewport({ scale });
  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.floor(vp.width));
  canvas.height = Math.max(1, Math.floor(vp.height));
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("无法创建画布");
  await page.render({ canvas, viewport: vp }).promise;
  return canvas.toDataURL("image/jpeg", 0.85);
}
