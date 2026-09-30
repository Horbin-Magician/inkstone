# 架构与关键决策

## 依赖与许可

2026-09-28 核对 crates.io 发布索引、实际下载的 Cargo.toml 与源码：

- GPUI Component / Base / Kit `=0.7.0`，共同使用 `gpui-pre =0.3.7`。
- `Cargo.lock` 提交版本控制；`cargo tree -i gpui-pre --locked` 确认只有一个 GPUI 版本，测试支持也复用它。
- 上述主要 UI crate 采用 Apache-2.0；没有引入或复制 Zed GPL editor crate。
- 笔记功能通过公开接口扩展。实测发现 Unicode 边界问题后，对 gpui-base 0.7.0 维护六个文件的受控补丁，详见 [补丁说明](EDITOR_PATCH.md)；所有调用仍共用同一 Base 和 GPUI 类型。

参考：[官方仓库](https://github.com/longbridge/gpui-kit)、[发布索引](https://index.crates.io/gp/ui/gpui-component)、[编辑接口](https://docs.rs/gpui-component/0.7.0/gpui_component/input/index.html)。下载的组件对应上游提交 `0c830f4d257e69fdd17200650533ab4ca9a40cc0`。

## 模块

| 模块 | 职责 |
| --- | --- |
| vault.rs | 文件扫描、路径验证、安全保存、恢复记录、重命名与回收区 |
| workspace/document.rs | 打开文档的保存状态与共享正文；编辑、选区和撤销统一使用 GPUI 编辑器 |
| markdown.rs | 限定实时样式扫描；输出 UTF-8 源码范围，不重写正文 |
| index.rs | Markdown AST、大纲、双链、路径解析、全文与文件名搜索、反链 |
| editor.rs | GPUI 编辑器、装饰、阅读视图、异步解析版本检查 |
| editor_links.rs | 公开 DefinitionProvider / CompletionProvider 扩展、补全过期校验 |
| workspace.rs | 窗口、标签、异步文件任务和原生监听事件调度 |
| workspace/commands.rs | 稳定命令编号、命令搜索、编辑操作与执行分发；菜单按编号查询，不按数组位置访问 |
| workspace/ui.rs | 工作区、侧栏、菜单和弹窗的界面组装 |

Markdown 文件是已保存正文事实来源。索引只在内存，可从文件完整重建。恢复记录是显式草稿副本，不混入正文。

## 编辑器路线

比较三个方向：

1. 公开接口扩展：复用 IME、文本选择和历史；TextDecorationCollection 提供实时样式，DefinitionProvider / CompletionProvider 提供双链交互。
2. 受控组件分支：修复软换行和光标的字素边界，并提供保留选区的光标滚入视口接口。需要维护跨块 Unicode 回归与原生几何测试。
3. 独立编辑器：自由度最高，需要重新实现输入、撤销、选择和平台接口，首版成本与风险更高。

当前实时预览通过组件的零宽标记跨度隐藏非活动语法，标题按级别设置字号与行高；正文、选区和撤销仍使用原始 Markdown 坐标。图片和表格等复杂块使用源码与独立阅读视图。

## 位置与异步边界

- Markdown / 编辑范围：UTF-8 字节半开区间，保留换行和原始格式。
- IME：EntityInputHandler 使用 UTF-16；直接集成测试覆盖 emoji 代理对与组词。
- 字素：unicode-segmentation 覆盖 ZWJ emoji、组合附加符号；实际光标几何仍由组件处理。
- 组件内部 LSP Position 的字符列通过 RopeExt 转换，不能直接当作 UTF-16 IME 范围。
- 可见位置、软换行、候选窗：组件 DisplayMap / 平台文本系统负责，多 DPI 仍待实测。
- Markdown 样式与 AST 在后台解析；安装结果时比较代次及当前原文。阅读组件也有后台解析和版本检查。
- 搜索绑定查询代次、笔记库代次。打开笔记绑定导航代次。外部重载比较路径、baseline 与保存状态。
- 普通文件事件只更新相关索引项；目录变动重建。索引复制与解析在后台。目录树用组件虚拟列表，目录列表改变时才重建树。
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

删除使用同卷回收区；Windows 重命名用 MoveFileW，不覆盖已有目标。Unix 路线保留备份但尚未经过平台验收。

## 当前 GPUI 判断

原生笔记闭环、阅读、搜索和基础输入有运行证据。长段落测试发现了组件原生的字素拆分问题，现已采用受控补丁，并承担后续升级回归成本。可以继续使用 GPUI 推进 MVP；多 DPI、完整原生光标回归和交互性能完成前，不能宣称生产级编辑体验。


## 阅读来源映射与双视图

`workspace/views.rs` 管理双视图，每份打开的文件只使用一个正文保存者与撤销栈；镜像视图保留自己的选区、滚动和阅读模式。输入法预编辑不复制到另一视图。

`rendering.rs` 将原始 Markdown 转为阅读展示，并保存原文件位置到展示文本的映射。嵌入展开后的链接携带来源路径，任务携带来源位置和文本快照。更新任务时检查快照，修改打开文档的正文或通过 Vault 安全保存关闭的来源文件；不会将展示用的链接协议或 HTML 写入笔记。闭合来源的写入计入退出等待，提交后立即更新匹配版本的索引。

TextView 的可选任务回调只报告位置与目标状态；源文本修改和冲突处理留在应用层。源偏移到阅读块位置的查询用于保持阅读模式的大纲/块跳转。
