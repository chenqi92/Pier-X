你在 Pier-X 仓库的一个 git worktree 里工作，分支 panel/logs，
目录 E:\workspace-freq\pier-x-logs\pier-ui-gpui。这是 Pier-X 的 GPUI（Rust）原生 UI 重写版，
基线提交 1ad739b 已经搭好面板系统。

【硬性规则】
- 只允许修改 src/panels/logs.rs 这一个文件。不要碰任何其他文件
  （panels/mod.rs、shell.rs、ui.rs、data.rs、Cargo.toml 都已接好）。
  如果你觉得需要改别的文件，说明思路错了——先停下来问。
- 你的面板已经注册好，对应工具被选中时会自动显示。它是一个 gpui View：
  pub struct LogsPanel、pub fn new(cx: &mut Context<Self>) -> Self、impl Render。
  把它们填充完整即可。
- 只用 `cargo build`（在 E:\workspace-freq\pier-x-logs\pier-ui-gpui 下）验证能编译通过。
  禁止启动程序、禁止截图、禁止运行 .exe。运行 GUI 的验证会在最后统一进行。
- 颜色/字体/尺寸只能用设计令牌：通过 self.theme（crate::theme::Theme）和
  crate::ui 里的共享组件（icon、panel_header、section_label、meter、info_row、
  status_dot、empty_state、level_color）。禁止硬编码 hex/rgb 颜色、字体名、
  或已有令牌的像素值。背景：t.bg/surface/panel/panel_2；文字：t.ink/ink_2/muted/dim；
  边框：t.line/line_2；强调色：t.accent；状态色：t.pos/neg/warn/info；
  间距 t.sp1..sp6；等宽字体 t.mono。
- 先研究参考实现：src/shell.rs::monitor_panel（真实数据 + 1.5s 刷新循环）、
  src/shell.rs::git_panel（一次性真实数据）、其他 src/panels/*.rs（View 骨架）、
  以及 src/ui.rs（组件库）。
- 绝不能在 render 路径里阻塞。render 只负责绘制。阻塞/IO 操作放到后台任务里，
  结果存进 View 状态，再 cx.notify()。模板：
      cx.spawn(async move |this, cx| {
          let result = cx.background_executor()
              .spawn(async move { /* 这里放阻塞调用 */ }).await;
          let _ = this.update(cx, |this, cx| { this.state = Some(result); cx.notify(); });
      }).detach();
- pier-core 保持与 UI 无关；直接当普通 Rust 依赖调用。不要往 Cargo.toml 加新依赖。
- 提交信息风格：feat(gpui): implement logs panel，正文为客观事实条目，
  不要任何 AI/厂商署名，不要“优化/重构”这类主观词。

【本任务目标 — Logs 面板】
实时查看 Pier-X 自己的日志文件。路径用 pier_core::logging::log_file_path() 获取。
读取最后约 500 行（放后台，别在 render 里读），按级别着色（ERROR→t.neg、WARN→t.warn、
INFO→t.info、DEBUG/TRACE→t.muted），等宽字体，可滚动，最新在底部。
仿照 monitor_panel 做一个约 1s 的受控刷新循环。
头部用 ui::panel_header(t, "scroll-text", "LOGS", <行数>)。没有日志文件则 ui::empty_state。
完成后在 E:\workspace-freq\pier-x-logs\pier-ui-gpui 下 `cargo build` 确认通过，然后提交。
