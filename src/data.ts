// ============================================================
// 休闲时光 - 数据声明
//
// 所有数据项统一在此声明，DataManager 自动识别和管理。
// 任何存在于 localStorage 但未在此声明的 key，
// 都会在 DataManager 中显示为"未注册数据"——说明开发不规范。
// ============================================================

import { defineDataStore } from "@glbt/appkit-ui";

export const store = defineDataStore({
  version: "1.0.0",
  namespace: "leisure",
  categories: {
    ui: {
      label: "界面状态",
      onDelete: "allowed",
    },
    preference: {
      label: "用户偏好",
      onDelete: "warn",
    },
  },
  items: {
    /** 顶层模块标签页（漫画 / 视频 / 小说[预留]） */
    "module-tab": {
      storage: "localStorage",
      category: "ui",
      desc: "当前选中的模块标签页",
      default: "comics" as string,
    },
    /** 视频模块内部标签页（影片 / 演员 / 标签） */
    "video-tab": {
      storage: "localStorage",
      category: "ui",
      desc: "视频模块内当前选中的标签页",
      default: "home" as string,
    },
    /** 漫画阅读器默认宽度（百分比，0 表示自适应） */
    "reader-width": {
      storage: "localStorage",
      category: "preference",
      desc: "漫画阅读器图片宽度（0 = 自适应窗口）",
      default: "0" as string,
    },
    /** 漫画阅读无痕滚动（滚动到章节末尾自动衔接下一章） */
    "reader-seamless": {
      storage: "localStorage",
      category: "preference",
      desc: "漫画阅读无痕滚动（滚动到章节末尾自动衔接下一章）",
      default: "0" as string,
    },
    /** 漫画阅读滚轮步长（视口高度百分比，1-100） */
    "reader-scroll-step": {
      storage: "localStorage",
      category: "preference",
      desc: "漫画阅读滚轮步长（视口高度百分比）",
      default: "10" as string,
    },
    /** 快捷短语列表（JSON 字符串） */
    "shortcuts": {
      storage: "localStorage",
      category: "preference",
      desc: "自定义快捷短语配置",
      default: JSON.stringify([
        { id: "1", abbreviation: "sj", replacement: "%yyyy%-%MM%-%dd% %HH%:%mm%:%ss%", enabled: true },
      ]) as string,
    },
    /** 选封面弹窗滚轮步进（窗口高度百分比，1-100） */
    "cover-wheel-pct": {
      storage: "localStorage",
      category: "preference",
      desc: "选封面滚轮步进（窗口高度百分比）",
      default: "10" as string,
    },
    /** 漫画书架/章节列表滚动位置记忆（JSON map：shelf:<root> → 书架滚动；series:<id> → 章节列表滚动） */
    "comic-scroll": {
      storage: "localStorage",
      category: "ui",
      desc: "漫画书架/章节列表滚动位置记忆",
      default: "{}" as string,
    },
    /** 浏览器模式访问口令（api.ts 读写；桌面模式不使用） */
    "web-token": {
      storage: "localStorage",
      category: "ui",
      desc: "局域网访问口令（浏览器端保存）",
      default: "" as string,
    },
    /** 漫画书架当前选中的导入路径（切换标签/重启后恢复） */
    "comic-active-root": {
      storage: "localStorage",
      category: "ui",
      desc: "漫画书架选中的导入路径",
      default: "" as string,
    },
    /** 小说书架当前选中的导入路径（切换标签/重启后恢复） */
    "novel-active-root": {
      storage: "localStorage",
      category: "ui",
      desc: "小说书架选中的导入路径",
      default: "" as string,
    },
    /** 小说阅读字号（px） */
    "novel-font-size": {
      storage: "localStorage",
      category: "preference",
      desc: "小说阅读字号（像素）",
      default: "17" as string,
    },
  },
  databases: {
    "leisure.db": {
      storage: "db",
      category: "ui",
      desc: "应用主数据库（漫画/视频/小说索引与阅读进度）",
    },
  },
});
