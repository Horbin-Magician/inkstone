# 架构与编辑器决策

## 依赖核对（2026-09-28）

- [GPUI Kit 官方仓库](https://github.com/longbridge/gpui-kit)
- [crates.io 发布索引](https://index.crates.io/gp/ui/gpui-component)
- [公开编辑器接口](https://docs.rs/gpui-component/0.7.0/gpui_component/input/index.html)
- 实际下载的 `gpui-component 0.7.0` Cargo.toml：依赖 `gpui-pre =0.3.7`，`gpui-base 0.7.0`。上游提交 `0c830f4d257e69fdd17200650533ab4ca9a40cc0`。
- Component/Base 采用 Apache-2.0。未复制 Zed GPL editor 代码，未修改组件内部实现。
- `Cargo.lock` 纳入版本控制。GPUI 类型统一来自同一 `gpui-pre` 版本，不与 crates.io `gpui 0.2.2` 混用。

## 路线比较

1. **公开接口扩展（当前原型）**：EditorState 提供输入法、软换行、选区、剪贴板和撤销；TextDecorationCollection 提供字节范围样式。维护成本最低。
2. **受控组件分支**：可加入隐藏语法的布局映射，但需要同时维护鼠标命中、IME bounds、折行与选区渲染；尚无必要修改。
3. **独立 Markdown 编辑器**：可控制全部映射，但需要自行实现跨行选择、复杂字素移动和平台 IME；首版风险最高。

先采用 1，实际交互失败时重新评估。原型的“隐藏”只改变标记透明度，**保留其布局宽度**。因此没有隐藏字符造成的 source/display 长度变化，中文输入仍交由组件编辑同一原文。标题用粗体与颜色，不单独改变字号。它不等价于 Obsidian 的紧凑排版。是否足以满足限定实时预览仍须以运行验证为准。

## 坐标边界

- Markdown 范围、样式和编辑事务：UTF-8 字节，半开区间。
- IME：组件通过 EntityInputHandler 转换 UTF-16；应用核心提供严格转换测试，拒绝代理对内部位置。
- 字素：unicode-segmentation，emoji ZWJ / 组合附加符号不能拆分为普通左右移动单位。
- 视觉坐标、软换行、候选框和选区几何：组件 DisplayMap 与平台文本布局负责；必须独立做 Windows 实测，核心转换测试不能代替。
- 原型不重新序列化 Markdown；复杂嵌套、多反引号代码和复杂块保持源码路径。

## 计划的职责边界

- document：文本事务、版本号、Unicode 工具；组件管理可见编辑器的撤销历史，避免双重撤销。
- markdown：源码范围与限定样式，后续独立后台解析阅读视图和链接。
- vault：文件扫描、冲突、安全写入、恢复区、外部变化。
- index：可重建搜索与反链，异步结果绑定库代次和文档版本。
- UI：GPUI 实体与动作，文件和较重解析任务通过后台执行器调度。

原型样式扫描在后台执行，结果校验解析代次；缓存文本和样式范围，光标变化仅重算激活状态。当前仍有 UI 线程全文字符串快照成本，长文档须测量后再优化。
