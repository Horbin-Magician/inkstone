# 架构与关键决策

## 依赖与许可

2026-09-28 核对 crates.io 发布索引、实际下载的 Cargo.toml 与源码：

- GPUI Component / Base / Kit `=0.7.0`，共同使用 `gpui-pre =0.3.7`。
- `Cargo.lock` 提交版本控制；`cargo tree -i gpui-pre --locked` 确认只有一个 GPUI 版本，测试支持也复用它。
- 上述主要 UI crate 采用 Apache-2.0；没有引入或复制 Zed GPL editor crate。
- 笔记功能通过公开接口扩展。实测发现 Unicode 边界问题后，对 gpui-base 0.7.0 维护六个文件的受控补丁，详见 [补丁说明](EDITOR_PATCH.md)；所有调用仍共用同一 Base 和 GPUI 类型。

参考：[官方仓库](https://github.com/longbridge/gpui-kit)、[发布索引](https://index.crates.io/gp/ui/gpui-component)、[编辑接口](https://docs.rs/gpui-component/0.7.0/gpui_component/input/index.html)。下载的组件对应上游提交 `0c830f4d257e69fdd17200650533ab4ca9a40cc0`。

## Cargo workspace

根 `Cargo.toml` 是虚拟 workspace，包含 `crates/inkstone-core` 与 `crates/inkstone-desktop` 两个默认成员，使用 resolver 3。共享依赖版本、release profile 与 `[patch.crates-io]` 只在根配置；`vendor/gpui-base`、`vendor/gpui-component` 显式排除出 workspace 成员，继续作为补丁依赖维护。

`inkstone-core` 提供 `inkstone_core` 库，拥有原 `lib.rs` 的全部核心模块；运行、构建和测试依赖均不包含 GPUI。`notify` 在 core 中只用于文件监听测试，平台安全文件操作的 `libc` / `windows-sys` 由 core 持有。Markdown 测试夹具与后台 benchmark 随核心代码放置。

`inkstone-desktop` 单向依赖 core，拥有原 `main.rs` 的桌面模块，二进制目标仍为 `inkstone`。GPUI、原生菜单和应用图标依赖只由 desktop 声明；字素补丁与展示对象集成测试也归入 desktop。macOS build script 在 desktop 内定位根目录 `packaging/macos/Info.plist`，打包脚本 `packaging/macos/bundle.sh` 输出根 `target/` 下的应用；正式图标放在 `crates/inkstone-desktop/assets/`，候选设计归入 `docs/design/branding/`。

核心层不得引用 desktop 或 GPUI 类型。共享类型和不依赖 UI 的算法放入 core，涉及 `Entity`、`Window`、焦点和原生图像的适配留在 desktop。包不独立发布，资源和受控依赖按整个仓库构建。

## 模块

下表核心模块路径相对于 `crates/inkstone-core/src/`，`editor`、`editor_links` 和 `workspace` 路径相对于 `crates/inkstone-desktop/src/`。

| 模块 | 职责 |
| --- | --- |
| vault/sync.rs、vault/sync/webdav.rs | 三方文件比较、内容校验、设备基线、WebDAV 条件提交与安全本地替换 |
| workspace/cloud_sync.rs | 同步配置、后台任务、保存等待与完成后的文档刷新 |
| vault.rs | 文件扫描、路径验证、安全保存、恢复记录、重命名与回收区 |
| workspace/document.rs | 打开文档的保存状态与共享正文；编辑、选区和撤销统一使用 GPUI 编辑器 |
| markdown.rs | 限定实时样式扫描；输出 UTF-8 源码范围，不重写正文 |
| index.rs | 索引快照、构建与增量刷新；对外保留解析类型和函数入口 |
| index/parsing.rs | Markdown AST 元数据、大纲、折叠范围、锚点与任务源码修改 |
| index/links.rs | 双链 / Markdown 路径解析、反链与移动引用更新计划 |
| index/search.rs | 文件名与别名排名、全文搜索、排序及结果上限 |
| editor.rs | 编辑器状态、事件、初始化与引用上下文 |
| editor/editing.rs | Markdown 按键、配对、折叠、行编辑与任务修改 |
| editor/presentation.rs | 解析版本检查、装饰和实时预览投影更新 |
| editor/reading.rs | 阅读状态恢复、焦点与源码位置跳转 |
| editor/view.rs | GPUI 视图组装与输入事件路由 |
| editor/counts.rs | 正文 / 选区字数缓存与异步统计 |
| editor_links.rs | 公开 DefinitionProvider / CompletionProvider 扩展、补全过期校验 |
| workspace.rs | 工作区共享状态、动作定义与轮询调度 |
| workspace/session.rs | 工作区初始化、笔记库选择 / 加载与会话恢复 |
| workspace/file_sync.rs | 外部文件刷新、索引与目录树同步 |
| workspace/note_files.rs | 新建、快速记录、重命名、回收及恢复副本 |
| workspace/note_open.rs | 打开笔记、选择目标视图与恢复视图状态 |
| workspace/tabs.rs | 标签创建、激活、关闭与等待保存后关闭 |
| workspace/saving.rs | 保存调度、恢复日志与冲突另存副本 |
| workspace/search.rs | 搜索焦点、后台查询结果与分页 |
| workspace/links.rs | 双链 / Markdown 链接导航与位置跳转 |
| workspace/commands.rs | 稳定命令编号、命令搜索、编辑操作与执行分发；菜单按编号查询，不按数组位置访问 |
| workspace/ui.rs | 工作区、侧栏、菜单和弹窗的界面组装 |

### 模块边界约定

桌面层使用 `Workspace` 和 `EditorPane` 作为 GPUI 实体及共享状态拥有者，子模块按职责实现其方法。子模块不复制文档状态，也不建立第二套保存、撤销或后台任务机制；跨子模块调用使用 `pub(super)`，不扩大应用外部接口。涉及多个功能的回归测试分别放在 `workspace/tests.rs`、`editor/tests.rs`，保留原来的测试名称和筛选路径；既有功能模块的局部测试继续就地维护。

核心层 `index` 不依赖桌面工作区。解析、搜索和链接模块通过 `Index` 快照与解析结果协作；`inkstone_core::index::parse`、`inkstone_core::index::ParsedNote` 等入口由父模块重新导出，调用方无需知道实现文件位置。索引回归放在 `index/tests.rs`。

新增功能优先放入对应职责模块：保存及恢复日志改动进入 `saving`，外部磁盘变化进入 `file_sync`，实时样式进入 `presentation`，索引搜索算法进入 `index/search`。顶层文件负责状态与组装，避免重新堆积功能实现。底层 Vault 的平台原子写入实现和 vendor 编辑器补丁沿用原有边界。

Markdown 文件是已保存正文事实来源。运行时索引驻留内存；应用数据目录中的启动缓存可丢弃，可从文件完整重建。缓存通过正文 SHA-256 校验，不能只依据大小和修改时间复用。恢复记录是显式草稿副本，不混入正文。

### 索引的用途与维护成本

- 路径、文件名和别名查找表用于链接解析和同名歧义判断；最短链接生成复用文件名查找表，不另建全库计数表。反链表避免每次展示反链都扫描全部笔记。
- 索引快照共享正文、解析结果和查找表。正文相同时不重新解析；仅正文（链接目标和别名不变）或文件时间变化时保留名称表和反链；链接目标变化只调整该笔记的反链。新增、删除、重命名和别名变化会影响其他笔记的解析，仍需重建反链。
- 全文搜索保留原有子串、正则及结构化条件语义，没有新增倒排索引或持久化数据库。搜索先过滤未达上限时的不匹配笔记，已有足够匹配笔记后延迟其余匹配工作，按展示顺序取足结果即停止；少量结果和无结果查询避免排序全库。快速打开只对前 200 项排序并创建结果对象。
- 文件内容和时间从同一个已验证的文件句柄读取；写缓存借用解析结果并缓冲输出，不深复制全库解析结果，也不先生成完整 JSON 字节数组。
- 链接补全仅在笔记路径、别名、标题名称、块 ID、附件列表或链接偏好变化时失效；普通正文和位置变化复用已有补全列表。快照比较仍需遍历笔记引用，但避免逐个打开视图重建全库补全对象。

全文搜索无匹配时仍需扫描全部正文，结构变化仍可能重建全部反链，快照的笔记目录仍有 O(N) 复制成本。是否增加全文倒排索引，应以后续大库的查询耗时、内存和更新成本实测决定；当前先降低现有索引的重复维护开销。测量条件和结果见 [验证指南](TESTING.md)。

## 编辑器路线

比较三个方向：

1. 公开接口扩展：复用 IME、文本选择和历史；TextDecorationCollection 提供实时样式，DefinitionProvider / CompletionProvider 提供双链交互。
2. 受控组件分支：修复软换行和光标的字素边界，并提供保留选区的光标滚入视口接口。需要维护跨块 Unicode 回归与原生几何测试。
3. 独立编辑器：自由度最高，需要重新实现输入、撤销、选择和平台接口，首版成本与风险更高。

实时预览通过零宽标记隐藏非活动文字语法，标题按级别设置字号与行高；复杂内容使用独立 `DisplayObject` 投影，首行预留宽度和测量高度，其余源行只在展示层隐藏。图片、表格、Callout、嵌入、公式和图表复用阅读片段或后台图形资源；光标或选区进入时展开源码。正文、复制、选区和撤销继续使用原始 Markdown 坐标。

`syntax::Snapshot` 复用同一版本的语法树、注释遮罩和行内脚注范围；公式与代码不贡献伪双链、任务、标签或标题。普通金额重新按正文解析，保留其链接与脚注。阅读片段保留完整来源文件快照，交互执行前检查版本。

公式与 Mermaid 使用纯 Rust 适配器输出 SVG，GPUI 按 DPI 准备原生图像。打开笔记时只排版全部公式以预留尺寸，字形轮廓和栅格化仅针对当前视口及相邻一屏，后台每批最多 4 个；滚动到附近后再继续。服务在视图间共享最多 64 MiB 图像缓存和 4096 条排版结果。阅读通过 MarkdownPlugin 读取缓存，缺图时通知编辑器补栅格。字形轮廓嵌入，中文回退到系统字体。展示坐标与文件字节之间的映射不会写回笔记。

## 位置与异步边界

- Markdown / 编辑范围：UTF-8 字节半开区间，保留换行和原始格式。
- IME：EntityInputHandler 使用 UTF-16；直接集成测试覆盖 emoji 代理对与组词。
- 字素：unicode-segmentation 覆盖 ZWJ emoji、组合附加符号；实际光标几何仍由组件处理。
- 组件内部 LSP Position 的字符列通过 RopeExt 转换，不能直接当作 UTF-16 IME 范围。
- 可见位置、软换行、候选窗：组件 DisplayMap / 平台文本系统负责，多 DPI 仍待实测。
- Markdown 样式与 AST 在后台解析；安装结果时比较代次及当前原文。阅读组件也有后台解析和版本检查。
- 搜索绑定查询代次、笔记库代次。打开笔记绑定导航代次。外部重载比较路径、baseline 与保存状态。
- 普通文件事件只更新相关索引项；目录创建或导入时扫描目录并增量刷新，完整重扫请求重建索引。删除条目通过索引维护入口同步清理名称表和反链。索引复制与解析在后台。索引快照通过 `Arc<IndexedNote>` 共享未变化的正文及解析结果，修改正文或文件时间时只替换对应笔记，避免每次监听事件深复制全库内容。目录树用组件虚拟列表，目录列表改变时才重建树。
- 反链使用虚拟列表，最多占四行高度，全部结果仍可滚动访问；避免万条反链把编辑器挤出窗口。列表改变时重置滚动位置。
- 标签切换、关闭标签和源码 / 阅读切换显式把焦点交给当前视图；各标签编辑实体继续保留历史和选区。

## Windows 保存

1. 先把 baseline 与 draft 写入应用恢复区并 sync_all。
2. 检查磁盘版本；持有允许读和删除、禁止其他写访问的源文件句柄。已有外部写句柄会使保存失败，不继续替换。
3. 新字节写入同目录唯一临时文件并 sync_all。
4. 新建使用不覆盖 hard_link；已有文件用 ReplaceFileW，备份位于同一卷。
5. 比较备份文件身份与受保护句柄，同时比较 baseline。发生替换竞争，保留外部备份与草稿并报告冲突。
6. 成功后移除受保护的临时备份；恢复记录改为 `.saved`。失败记录保留 `.json`。

[Microsoft ReplaceFileW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew) 说明部分错误可能改变文件名称，且 WRITE_THROUGH 标志不受支持。实现保留备份和恢复日志，不宣称所有设备下绝对断电原子性。普通写错误会保留内存、显示标签错误并停止自动重试。冲突需要另存副本，不自动决定使用哪个版本。

删除使用同卷回收区；Windows 重命名用 MoveFileW，不覆盖已有目标。macOS 使用 `renameatx_np(RENAME_EXCL)`，Linux 使用 `renameat2(RENAME_NOREPLACE)`，文件和目录均拒绝覆盖已有目标。

macOS/Linux 保存先将草稿移到唯一备份名称，再通过 `RENAME_SWAP` / `RENAME_EXCHANGE` 原子交换目标与草稿；备份因此捕获实际被替换的文件，不存在先硬链接旧文件再覆盖新文件的窗口。交换后核对原文件身份和基线，冲突保留两者。Unix 保留原文件权限。文件系统不支持原子交换时明确报错，不退回不安全替换。Unix 外部程序持有旧文件句柄进行延迟原地写入仍不能由本应用禁止，备份保留用于恢复；Linux 与真实断电行为尚未在本轮验收。

## 当前 GPUI 判断

原生笔记闭环、阅读、搜索和基础输入有运行证据。长段落测试发现了组件原生的字素拆分问题，现已采用受控补丁，并承担后续升级回归成本。可以继续使用 GPUI 推进 MVP；多 DPI、完整原生光标回归和交互性能完成前，不能宣称生产级编辑体验。


## 阅读来源映射与双视图

`workspace/views.rs` 管理双视图，每份打开的文件只使用一个正文保存者与撤销栈；镜像视图保留自己的选区、滚动和阅读模式。输入法预编辑不复制到另一视图。

`rendering.rs` 将原始 Markdown 转为阅读展示，并保存原文件位置到展示文本的映射。嵌入展开后的链接携带来源路径，任务携带来源位置和文本快照。更新任务时检查快照，修改打开文档的正文或通过 Vault 安全保存关闭的来源文件；不会将展示用的链接协议或 HTML 写入笔记。闭合来源的写入计入退出等待，提交后立即更新匹配版本的索引。

TextView 的可选任务回调只报告位置与目标状态；源文本修改和冲突处理留在应用层。源偏移到阅读块位置的查询用于保持阅读模式的大纲/块跳转。


## WebDAV 同步边界

核心 `Remote` 接口隔离传输与合并，首个实现是 reqwest blocking + rustls WebDAV。桌面端只在后台执行客户端初始化与网络请求；服务器配置进入 Preferences。密码不进入配置、清单或基线：macOS 使用登录钥匙串，Windows 使用凭据管理器，账号键是服务器、用户名与笔记库路径的 SHA-256。读取失败不会在随后的空密码保存中删除旧项。Linux 尚无钥匙串实现。同步时复用文件操作互斥和未完成写入计数，阻止库切换与提前退出。已配置且开启自动同步时，打开笔记库以及本地新建、保存或删除后会排队同步；文件变化先等待短暂静默，失败后退避重试。未配置或关闭自动同步时仍只在手动点击后同步。

远端内容对象不可变，清单通过强 ETag 和条件 PUT 发布；先校验全部下载，再发布合并清单，最后应用本地文件并推进设备基线。失败保留旧基线，允许重试；提交后本地应用不是整库事务，部分成功需要下一次同步完成。本地按内容哈希重新检查变化，替换或删除保留隐藏备份，不执行无恢复的删除。设备基线和进程文件锁放在库外应用恢复目录，避免同步状态自身参与同步。校验值保护内容完整性，不提供端到端加密。
