import { createViteConfig } from "@glbt/appkit-ui/vite-config";
export default createViteConfig({
  port: 5183,
  strictPort: false,
  // jassub 的 worker/wasm 需由 vite 打包（多文件 ESM worker），排除预打包由 vite 即时转换
  optimizeDepsExclude: ["jassub"],
});
