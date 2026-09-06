/// <reference types="vite/client" />

// @glbt/appkit-ui 的 package.json 仅导出 "./styles" 子路径（指向 CSS 文件）。
// 引入该子路径时 TypeScript 需要一条环境模块声明，否则 tsc 报 TS2882。
declare module "@glbt/appkit-ui/styles";
