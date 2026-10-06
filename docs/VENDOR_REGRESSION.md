# Vendor 补丁与回归入口

两个包均来自 crates.io 的 0.7.0 发布包，上游提交 `0c830f4d257e69fdd17200650533ab4ca9a40cc0`，分别对应 `crates/base` 和 `crates/component`。Apache-2.0 许可证保留在 vendor 包中。以下清单按 2026-10-05 对原始发布包的逐文件比较生成；路径均相对于各 vendor 包。新增文件没有对应上游文件。

## 独立运行

```sh
python3 tools/vendor-regression/run.py
```

脚本在 `target/vendor-regression` 中重新复制当前源码，保持上游 `crates/base` / `crates/component` 布局，并复制同一上游提交的 README、图标、Markdown 和主题夹具供 include_str/include_bytes 测试使用。测试执行两个包的全部 library tests，失败返回非零；不使用旧 target 源码作测试输入。`tools/vendor-regression/Cargo.lock` 固定其开发依赖，主 Cargo.lock 不会被改写。普通工作区测试不运行依赖包内部测试，不能替代此命令。

2026-10-06 在 macOS 独立源码目录、空 Cargo 依赖缓存及构建目录执行上述正式入口：Base 1,236 项、Component 572 项全部通过，无忽略项。暂存源码与归档的 vendor 源码一致，两份独立锁均未变化；仍复用本机工具链和 SDK，不能替代 Windows、远端 CI 或原生交互验收。详见 [验收记录](TESTING.md)。

CI 独立的 `Vendor regression` macOS/Windows jobs 执行同一命令。2026-10-06 已从实际 GitHub check runs 核实四个检查名称，并为 master 配置必需检查：`check (macos-latest)`、`check (windows-latest)`、`Vendor regression (macos-latest)`、`Vendor regression (windows-latest)`，均限定 GitHub Actions app（15368）。严格要求分支最新，管理员也受约束；远端读取复核与配置一致。待完成或失败的检查不能正常合入，配置门槛不等于测试已经通过；实际运行结果独立记录。修改 job 名称时必须同步更新远端配置，本地 YAML 不会自动更新门槛。

工作流不依赖第三方 Rust 缓存 action。最近远端运行在准备阶段无法解析 `Swatin/rust-cache`，因此移除这一可选依赖；后续 CI 构建可能更慢，工作区和 vendor 检查命令保持不变，核心回归包含在工作区测试中。支持 push、pull_request 和手动 workflow_dispatch；新工作流在独立分支触发运行 37419921824 后已通过初始化。用户随后明确不需要 Linux 适配，现仅保留 macOS/Windows 的工作区及 vendor 四项检查；提交 e96db5c（run 37429373295）的远端 macOS 和 Windows 工作区及 vendor 四项检查已全部通过；vendor 每平台 1,808 项（macOS Base 1,236/Component 572；Windows Base 1,237/Component 571），无忽略项。成功的检查名称/app id 已与主分支门槛逐项核对。

每个工作区和 vendor job 设置 45 分钟总上限，包含工具链、依赖编译及测试；超时按失败处理，不忽略测试或自动重跑。已有成功的冷构建任务约 9–13 分钟，上限留有余量；它是 CI 资源边界，不是编辑性能预算。修改工作流只影响随后触发的运行，不会中断已经运行的任务。

## 维护目的与验证层次

| 补丁范围 | 目的 | 验证入口 |
| --- | --- | --- |
| Base input/base、input/editor | 字素光标、IME、共享撤销、多光标、Markdown 编辑、显示投影、布局缓存与滚动 | Base library 全量；应用 grapheme_cursor、grapheme_wrap、display_objects 集成测试及 editor/workspace 回归 |
| Base text | Markdown/HTML 源位置、任务与链接、脚注、Callout、选择、文字布局 | Base library 全量；应用阅读/实时预览回归 |
| Base tree 与 Component tree/resizable | 文件树选择与可控工作区布局 | 两包 library 全量；应用文件树与分屏回归 |
| Component input | 编辑器属性透传、补全菜单排版、Unicode 高亮、菜单定位 | Component library 全量；应用补全回归 |
| Component menu | 点击修饰键透传、长按与拖拽菜单、实际事件传递；自定义菜单行通过 `element_with_label` 提供交互节点的可访问名称 | Component library 全量；`popup_menu_item_a11y_label_uses_visible_label`；workspace navigation 回归 |
| Base/Component slider | 单值滑块键盘焦点、方向/边界键、用户调整事件及可访问名称 | 两包 library 全量（slider::tests）；应用 settings_sliders_accept_keyboard_changes_and_keep_focus |
| Component switch/title_bar | 设置开关尺寸、标题栏内容收缩 | Component library 全量；设置滚动/点击回归和原生视觉验收 |

具体行为与升级注意事项见 [编辑补丁说明](EDITOR_PATCH.md) 和 [Component 补丁说明](../vendor/gpui-component/INKSTONE_PATCHES.md)。自动测试不替代原生候选窗、DPI、滚动与输入延迟验收。

## gpui-base 源码清单


修改的上游文件：

- `src/slider.rs`（2026-10-06 补充）


- `src/input/base/cursor.rs`

- `src/input/base/element.rs`

- `src/input/base/layout.rs`

- `src/input/base/mode.rs`

- `src/input/base/movement.rs`

- `src/input/base/state.rs`

- `src/input/base/undo_manager.rs`

- `src/input/editor/auto_close.rs`

- `src/input/editor/display_map/display_map.rs`

- `src/input/editor/display_map/fold_map.rs`

- `src/input/editor/display_map/inline_line.rs`

- `src/input/editor/display_map/mod.rs`

- `src/input/editor/display_map/text_wrapper.rs`

- `src/input/editor/display_map/wrap_map.rs`

- `src/input/editor/highlighting.rs`

- `src/input/editor/indent.rs`

- `src/input/editor/language.rs`

- `src/input/editor/language_config.rs`

- `src/input/editor/lsp/completions.rs`

- `src/input/editor/lsp/definitions.rs`

- `src/input/editor/lsp/mod.rs`

- `src/input/editor/mod.rs`

- `src/input/editor/search.rs`

- `src/input/mod.rs`

- `src/text/format/html.rs`

- `src/text/format/markdown.rs`

- `src/text/inline_element.rs`

- `src/text/markdown_ext.rs`

- `src/text/node.rs`

- `src/text/range_highlight.rs`

- `src/text/selection_adapter.rs`

- `src/text/state.rs`

- `src/text/stream_fade.rs`

- `src/text/style.rs`

- `src/text/text_view.rs`

- `src/tree.rs`



新增补丁文件：


- `src/input/base/grapheme_cursor.rs`

- `src/input/base/occurrences.rs`

- `src/input/editor/line_typography.rs`

- `src/input/editor/concealment.rs`

- `src/input/editor/display_objects.rs`

- `src/input/editor/display_map/grapheme_wrap.rs`



## gpui-component 源码清单


修改的上游文件：

- `src/slider.rs`（2026-10-06 补充）


- `src/input/editor.rs`

- `src/input/input.rs`

- `src/input/popovers/completion_menu.rs`

- `src/input/state.rs`

- `src/menu/context_menu.rs`

- `src/menu/menu_item.rs`

- `src/menu/popup_menu.rs`

- `src/resizable.rs`

- `src/switch.rs`

- `src/title_bar.rs`

- `src/tree.rs`



新增补丁文件：


无。



## 更新方式

1. 修改 vendor 前对照上述上游路径与既有回归；更新后运行主工作区检查及本页独立命令。
2. 更换包版本时逐项确认补丁是否已被上游覆盖，更新来源、文件清单、许可证与测试证据；不要整体覆盖后只验证应用能编译。
3. 需要更新开发依赖时先 `python3 tools/vendor-regression/run.py --prepare-only`，在生成的工作区执行有意的 `cargo update --manifest-path target/vendor-regression/Cargo.toml`，再将生成的 Cargo.lock 复制回 `tools/vendor-regression/Cargo.lock`；复查与应用锁的版本差异后执行完整独立回归。不得在 CI 自动更新锁。

独立锁的共同依赖版本差异：测试开发依赖增加 itertools 0.10.5；其构建依赖约束将独立测试的 cc 固定为 1.2.67（应用仍为 1.5.1）。phf_macros 0.11.3 仅由应用依赖使用，未出现在独立锁中；其余共同包版本一致。两种依赖图均需保留回归，不能用独立测试取代应用集成测试。
