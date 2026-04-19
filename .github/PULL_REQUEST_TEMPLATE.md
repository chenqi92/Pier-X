<!--
Thanks for contributing to Pier-X. Keep this short and to the point.
Delete any sections that don't apply to your change.
-->

## 改动说明 / Summary

<!--
What does this PR do, and why? One or two paragraphs.
Link the driving issue if there is one: Fixes #123 / Refs #456.
-->

## 影响面 / Scope

- [ ] `pier-core` (Rust backend, cross-platform)
- [ ] `pier-ui-gpui` (desktop shell)
- [ ] 构建 / 打包脚本 (`scripts/`)
- [ ] CI / release workflows (`.github/`)
- [ ] 设计规范 / 文档 (`.agents/skills/pier-design-system/`, `docs/`)
- [ ] 本地化 (`locales/app.yml`)

## 验证 / Verification

<!--
What did you run to convince yourself this works?
Paste commands and (where relevant) screenshots.
CI runs cargo fmt/clippy/test automatically — call out any
manual / visual verification here.
-->

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings`
- [ ] `cargo test -p pier-core --release --locked`
- [ ] UI 目视验证（浅色 + 深色主题，截图附上）
- [ ] 跨平台验证（如涉及平台特异代码）

## 风险 / Risk

<!--
Anything reviewers should pay special attention to?
Non-obvious regressions, performance, migration of persisted state, etc.
-->

## 遵守项 / Checklist

- [ ] 没有新增的 `rgb()` / `rgba()` / `px(<裸数字>)` / 硬编码字体字符串进入 `views/` 或 `app/`（[CLAUDE.md](../CLAUDE.md) Rule 1）
- [ ] 新增的视觉原子已封装成 `src/components/<name>.rs`（Rule 2）
- [ ] 渲染函数体内没有 IO / `_blocking` 调用（Rule 6）
- [ ] 没有重新引入 Tauri / Qt / npm / cmake 任何痕迹
