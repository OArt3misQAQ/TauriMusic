import js from "@eslint/js";
import pluginVue from "eslint-plugin-vue";
import globals from "globals";
import tseslint from "typescript-eslint";

export default tseslint.config(
  {
    // html/ 与 web/ 为本地宣传页源码,已 gitignore,不参与仓库 lint
    ignores: [
      "dist/**",
      "src-tauri/**",
      "node_modules/**",
      "scripts/**",
      "TauriMusic-promo/**",
      "html/**",
      "web/**",
    ],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  ...pluginVue.configs["flat/recommended"],
  {
    files: ["**/*.vue"],
    languageOptions: {
      parserOptions: { parser: tseslint.parser },
      globals: { ...globals.browser },
    },
  },
  {
    languageOptions: {
      globals: { ...globals.browser },
    },
    rules: {
      // 组件均为单名词(Modal/Sidebar/TopBar...),与 Apple Music 风格命名一致
      "vue/multi-word-component-names": "off",
      // 模板中长属性行不强制换行
      "vue/max-attributes-per-line": "off",
      "vue/singleline-html-element-content-newline": "off",
      "vue/html-self-closing": "off",
      "vue/html-indent": "off",
      "vue/html-closing-bracket-newline": "off",
      "vue/attributes-order": "off",
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
      ],
      "@typescript-eslint/no-explicit-any": "warn",
    },
  },
  {
    // 环境声明文件里的样板写法(DefineComponent<{}, {}, any>)不做约束
    files: ["**/*.d.ts"],
    rules: {
      "@typescript-eslint/no-empty-object-type": "off",
      "@typescript-eslint/no-explicit-any": "off",
    },
  },
);
