// ============================================================
// ESLint 配置 — 继承共享配置
// ============================================================

import sharedConfig from "../../shared/eslint.config.mjs";

export default [
  ...sharedConfig,
  {
    // 项目特定的规则可在此追加
  },
];
