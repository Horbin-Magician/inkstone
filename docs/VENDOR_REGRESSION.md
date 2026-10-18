# Vendor 补丁与回归入口

两个包均来自 crates.io 的 0.7.0 发布包，上游提交 `0c830f4d257e69fdd17200650533ab4ca9a40cc0`，分别对应 `crates/base` 和 `crates/component`。Apache-2.0 许可证保留在 vendor 包中。以下清单按 2026-10-05 对原始发布包的逐文件比较生成；路径均相对于各 vendor 包。新增文件没有对应上游文件。

## 独立运行

```sh
python3 tools/vendor-regression/run.py
```

脚本在 `target/vendor-regression` 中重新复制当前源码，保持上游 `crates/base` / `crates/component` 布局，并复制同一上游提交的 README、图标、Markdown 和主题夹具供 include_str/include_bytes 测试使用。测试依次执行 Base 默认配置、Base 启用 `test-support` 配置、Component 默认配置和 Component 启用 `test-support` 配置的全部 library tests，失败返回非零；不使用旧 target 源码作测试输入。`tools/vendor-regression/Cargo.lock` 固定其开发依赖，主 Cargo.lock 不会被改写。普通工作区测试不运行依赖包内部测试，不能替代此命令。

2026-10-06 在 macOS 独立源码目录、空 Cargo 依赖缓存及构建目录执行上述正式入口：Base 1,236 项、Component 572 项全部通过，无忽略项。暂存源码与归档的 vendor 源码一致，两份独立锁均未变化；仍复用本机工具链和 SDK，不能替代 Windows、远端 CI 或原生交互验收。详见 [验收记录](TESTING.md)。

CI 独立的 `Vendor regression` macOS/Windows jobs 执行同一命令。2026-10-06 已从实际 GitHub check runs 核实四个检查名称，并为 master 配置必需检查：`check (macos-latest)`、`check (windows-latest)`、`Vendor regression (macos-latest)`、`Vendor regression (windows-latest)`，均限定 GitHub Actions app（15368）。严格要求分支最新，管理员也受约束；远端读取复核与配置一致。待完成或失败的检查不能正常合入，配置门槛不等于测试已经通过；实际运行结果独立记录。修改 job 名称时必须同步更新远端配置，本地 YAML 不会自动更新门槛。

工作流不依赖第三方 Rust 缓存 action。最近远端运行在准备阶段无法解析 `Swatin/rust-cache`，因此移除这一可选依赖；后续 CI 构建可能更慢，工作区和 vendor 检查命令保持不变，核心回归包含在工作区测试中。支持 push、pull_request 和手动 workflow_dispatch；新工作流在独立分支触发运行 37419921824 后已通过初始化。用户随后明确不需要 Linux 适配，现仅保留 macOS/Windows 的工作区及 vendor 四项检查；提交 e96db5c（run 37429373295）的远端 macOS 和 Windows 工作区及 vendor 四项检查已全部通过；vendor 每平台 1,808 项（macOS Base 1,236/Component 572；Windows Base 1,237/Component 571），无忽略项。成功的检查名称/app id 已与主分支门槛逐项核对。

每个工作区和 vendor job 设置 45 分钟总上限，包含工具链、依赖编译及测试；超时按失败处理，不忽略测试或自动重跑。已有成功的冷构建任务约 9–13 分钟，上限留有余量；它是 CI 资源边界，不是编辑性能预算。修改工作流只影响随后触发的运行，不会中断已经运行的任务。

## 维护目的与验证层次

2026-10-19 对话框 Escape 开关：关闭 Escape 时使用保留 Enter 确认的按键上下文，显式取消动作保持有效；与上层 keyboard 仅控制 Escape 的文档一致。

2026-10-19 对话框触发器去重：AlertDialogTrigger 委托 DialogTrigger 的指针打开行为，保留 handle 与 on_open 接口，实际点击回归同时覆盖两种触发器。

2026-10-19 DatePicker 禁用语义：真实 ComboBox 节点报告 disabled，保留展开状态；动作回归明确禁用时禁止确认，但仍允许取消已展开的选择器。

2026-10-19 DatePicker 焦点绑定：显式 track_focus 更新渲染使用的句柄，避免被构造句柄覆盖；真实焦点遍历回归覆盖覆盖关系。

2026-10-19 单侧 DropdownButton：仅当操作按钮和菜单按钮同时存在时才应用拼接边框/圆角，单侧按钮保留完整外缘。

2026-10-19 Toggle 禁用语义：Base Toggle 复用透明无障碍适配器，让实际节点报告 disabled；维持选中状态、焦点策略及禁用时无 Click action，测试读取真实角色与绘制后的观察属性。

2026-10-19 ToggleGroup 激活：子控件直接向组报告 checked 状态，支持 Enter/Space；没有组回调时保留子控件原回调。移除父容器点击转发及临时索引，组与子控件禁用状态仍合并生效。

2026-10-19 按钮组激活：由子按钮直接报告选择结果，统一指针与 Enter/Space，移除父容器事件中转和最后点击索引缓存，避免空白区域点击误报或重放选择。

2026-10-19 按钮组禁用状态：渲染时合并组与子按钮自身的 disabled，统一 child/children 及构建顺序，避免禁用按钮仍可激活或获得 Tab 焦点。

2026-10-19 按钮忙碌语义：Base Button 新增 busy 状态，保持焦点和正常样式，阻止激活并向原生节点报告 busy/disabled、移除 Click action。Component loading 接入该状态，删除重复点击拦截及重复 cursor_default 设置；鼠标按下与父事件阻断保持回归覆盖。

2026-10-19 按钮禁用语义：Base Button 复用原生禁用适配器，禁用时 AccessKit 节点报告 disabled 且不暴露 Click action，保持原有指针/键盘阻断。原测试中记录上游缺口的断言改为要求真实禁用标记。

2026-10-19 原生禁用标记：新增 Base `src/accessibility.rs` 透明元素适配器，转发布局、标识、交互与无障碍子节点，并向真实 AccessKit 节点写入禁用标记。观察封装保留在适配器之外且保留焦点绑定；首个接入组件为 AccordionTrigger。

2026-10-19 多内容折叠展开：Base `src/collapsible.rs` 在 reveal ID 下按子元素索引区分各内容块的 MotionReveal，避免不同高度内容共享测量缓存。布局回归使用 20px/60px 内容和 50% 进度检查两个后续元素位置。

2026-10-19 悬停任务清理：Base `src/hover_card.rs` 安排延迟打开/关闭前复用 cancel_tasks 已递增的 epoch，删除重复递增和冗余辅助函数；旧任务失效策略保持一致。

2026-10-19 嵌套焦点容器：Base `src/focus_trap.rs` 按实际焦点树选择包含当前焦点的最内层容器，避免 HashMap 遍历顺序决定 Tab 约束范围。回归显式覆盖两种容器遍历顺序、外层焦点及失焦。

2026-10-19 关键帧播放收尾：Base `src/motion.rs` 保存减少动态效果对当前播放的完成状态，恢复动态效果不再跳回旧进度；显式更换播放 ID 可重新播放。回归检查中途收尾、恢复后终值与无待处理帧，以及新 generation 的正常进度。

2026-10-19 入场/退场状态：Base `src/motion/presence.rs` 将起点与目标相同的状态判为完成，初始隐藏不再挂载无意义的退场动画，减少动态效果关闭后已收尾状态保持稳定。回归检查无待处理帧、退出中途立即隐藏及后续正常入场。

2026-10-19 动画序列收尾：Base `src/motion/sequence.rs` 保存减少动态效果导致的完成状态，恢复动态效果时不再重新报告运行中或请求无效帧。回归同时检查后续目标变化仍从当前位置正常播放。

2026-10-19 虚拟列表自动尺寸：合并横向/纵向的重复测量分支，宽和高统一使用各自的 known_dimensions，移除横向高度误引用 width 的表达式。真实布局回归检查定宽横向列表的内容高度；该回归在清理前也通过，不作为旧表达式错误被触发的证据。

2026-10-19 虚拟列表缓存：Base `src/virtual_list.rs` 将方向和解析后的条目间距纳入尺寸缓存条件，条目不变时切换方向或间距也会重建前缀位置与内容范围。真实布局回归检查变化前后的内容长度与滚动到底偏移。

2026-10-19 自动滚动：Base `src/auto_scroll.rs` 将内部触发区限制为视口高度的一半，避免小于 32px 的视口上下触发区重叠；零高度不滚动。普通视口的速度曲线保持不变。回归检查窄视口中心静止、上下方向对称及零高度。

2026-10-10 加粗颜色：Base 的 `TextViewStyle::with_strong_color` 为加粗片段提供可选前景色，默认继承正文颜色；该配置参与样式比较以刷新缓存。`node::mark_highlight` 在保留粗体的同时应用颜色，行内代码和链接仍可覆盖前景色。Inkstone 的阅读视图和实时预览对象使用主题强调色，与实时正文加粗保持一致。

2026-10-07 代码块高亮：Component 的 `src/highlighter/languages.rs` 为 TSX 复用完整 TypeScript / JavaScript 查询，并新增 `src/highlighter/languages/typescript/jsx.scm`。原查询只覆盖 TypeScript 扩展，普通关键字及 JSX 没有颜色。专项回归 `tsx_highlights_javascript_types_and_jsx` 需开启 `tree-sitter-tsx`；应用另检查全部启用语言的深浅主题高亮及 UTF-8 范围。

2026-10-07 空闲重绘修复：`TextWrapper::set_inline_metrics` 将实际变化标志向上
传递至 `DisplayMap`，相同度量不再重建折叠映射及标题高度表。变化和清空仍重建；
新增回归用相同值的独立分配验证复用，并将变化后的映射和高度与全新构造的结果比较。

同日补充长段落范围查询：无折叠/隐藏行时直接使用 WrapMap 的范围和长度，避免逐个
软换行行扫描；有隐藏行的路径保持原逻辑。回归覆盖长 Unicode 段落、折叠/展开、
整行隐藏/恢复及越界行号，独立 ignored 微基准比较查询算法而非原生交互。

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

- `src/accessibility.rs`（2026-10-19）


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
