# 架构与关键决策

## 依赖与许可

2026-09-28 核对 crates.io 发布索引、实际下载的 Cargo.toml 与源码：

- GPUI Component / Base / Kit `=0.7.0`，共同使用 `gpui-pre =0.3.7`。
- `Cargo.lock` 提交版本控制；`cargo tree -i gpui-pre --locked` 确认只有一个 GPUI 版本，测试支持也复用它。
- 上述主要 UI crate 采用 Apache-2.0；没有引入或复制 Zed GPL editor crate。
- 当前完全通过公开接口扩展，未维护组件分支或修改组件内部实现。

参考：[官方仓库](https://github.com/longbridge/gpui-kit)、[发布索引](https://index.crates.io/gp/ui/gpui-component)、[编辑接口](https://docs.rs/gpui-component/0.7.0/gpui_component/input/index.html)。下载的组件对应上游提交 `0c830f4d257e69fdd17200650533ab4ca9a40cc0`。

## 模块

| 模块 | 职责 |
| --- | --- |
| vault.rs | 文件扫描、路径验证、安全保存、恢复记录、重命名与回收区 |
| document.rs | Unicode 边界工具、独立编辑事务模型与测试；可见编辑器使用组件自己的事务历史，不维护双重撤销 |
| markdown.rs | 限定实时样式扫描；输出 UTF-8 源码范围，不重写正文 |
| index.rs | Markdown AST、大纲、双链、路径解析、全文与文件名搜索、反链 |
| editor.rs | GPUI 编辑器、装饰、阅读视图、异步解析版本检查 |
| editor_links.rs | 公开 DefinitionProvider / CompletionProvider 扩展、补全过期校验 |
| workspace.rs | 窗口、标签、动作、目录树、异步文件任务和原生监听事件调度 |

Markdown 文件是已保存正文事实来源。索引只在内存，可从文件完整重建。恢复记录是显式草稿副本，不混入正文。

## 编辑器路线

比较三个方向：

1. 公开接口扩展：复用 IME、软换行、文本选择和历史；TextDecorationCollection 提供实时样式，DefinitionProvider / CompletionProvider 提供双链交互。当前采用。
2. 受控组件分支：可以消除隐藏标记宽度，但需维护布局、命中、IME bounds、选区和软换行的完整映射。当前尚未修改。
3. 独立编辑器：自由度最高，需要重新实现输入、撤销、选择和平台接口，首版成本与风险更高。

当前隐藏仅改变标记透明度，保留占位，因此显示布局使用同一源码坐标。标题使用粗体与颜色，不改变字号。该方案确实在原文编辑中提供实时样式，但不等同于消除语法宽度的紧凑实时预览。

## 位置与异步边界

- Markdown / 编辑范围：UTF-8 字节半开区间，保留换行和原始格式。
- IME：EntityInputHandler 使用 UTF-16；直接集成测试覆盖 emoji 代理对与组词。
- 字素：unicode-segmentation 覆盖 ZWJ emoji、组合附加符号；实际光标几何仍由组件处理。
- 组件内部 LSP Position 的字符列通过 RopeExt 转换，不能直接当作 UTF-16 IME 范围。
- 可见位置、软换行、候选窗：组件 DisplayMap / 平台文本系统负责，多 DPI 仍待实测。
- Markdown 样式与 AST 在后台解析；安装结果时比较代次及当前原文。阅读组件也有后台解析和版本检查。
- 搜索绑定查询代次、笔记库代次。打开笔记绑定导航代次。外部重载比较路径、baseline 与保存状态。
- 普通文件事件只更新相关索引项；目录变动重建。索引复制与解析在后台。目录树用组件虚拟列表，目录列表改变时才重建树。

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

公开接口已足以实现本轮功能，Windows 原型确实运行过，微软拼音基础组词与自动化 IME 测试有证据。继续使用 GPUI 是合理的工程选择；当前还不足以宣称编辑体验达到 Obsidian 或生产使用质量。是否需要分支，应由长文档、实际补全点击、多 DPI 和候选框回归结果决定。
