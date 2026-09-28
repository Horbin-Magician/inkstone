# 砚台 / Inkstone

Rust + GPUI 本地 Markdown 笔记应用。**当前是待完成桌面验收的 MVP 实现，目标尚未标记完成。** Windows release 已构建，自动测试通过；锁屏后新增界面的完整桌面回归仍待执行。

## 运行

已有可执行文件：`target/release/inkstone.exe`。从本仓库目录运行：

```powershell
.\target\release\inkstone.exe .\demo-vault
```

打开自己的笔记库：

```powershell
.\target\release\inkstone.exe "D:\我的笔记"
```

也可以不传参数，启动后用“打开笔记库”选择文件夹；应用记住最近的库。

## 构建与验证

测试环境：Windows 11、Rust 1.98.1、Visual Studio 2022 C++ Build Tools + Windows SDK。

```powershell
cargo build --release --locked --bin inkstone
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
cargo run --release --locked --example benchmark -- 10000
```

macOS / Linux 保留条件编译代码，尚未构建或验收。

## 已实现

- 文件夹笔记库、最近库、可展开目录树、创建（含子目录）、重命名、可恢复删除。
- 独立编辑状态的标签页、未保存标记、自动保存（2 秒）与手动保存。
- GPUI 原生源码编辑、限定实时样式、阅读预览；不使用 WebView。
- 阅读视图接入组件 Markdown 渲染器：标题、列表、任务列表、引用、代码块、链接、表格、本地图片；完整视觉验收待补。
- 文件名搜索、全文搜索（最多显示 200 个匹配）、文档内查找、大纲跳转、命令面板。
- 双链补全、Ctrl+Enter / Ctrl+点击定义跳转、阅读链接及出站双链点击、创建缺失目标、反向链接。
- Windows 原生文件监听；后台索引 / 解析 / 文件操作，异步结果校验。
- 保存前恢复记录、同目录安全替换、外部冲突阻止、另存副本、写失败后保留内存内容与错误。

## 键盘操作

| 快捷键 | 功能 |
| --- | --- |
| Ctrl+O | 打开笔记库 |
| Ctrl+N | 聚焦新笔记名称，Enter 创建 |
| Ctrl+P | 文件名快速打开，Enter 打开首个结果 |
| Ctrl+Shift+F | 全文搜索 |
| Ctrl+F | 当前文档查找 |
| Ctrl+S | 保存 / 重试失败保存 |
| Ctrl+W | 关闭已保存标签 |
| Ctrl+Shift+P | 命令面板 |
| Ctrl+Enter | 跳转光标所在双链 |
| Ctrl+Z / Ctrl+Y | 撤销 / 重做 |

补全候选由组件提供键盘选择。Ctrl+点击、补全菜单操作与上述快捷键的最新界面桌面回归尚未全部完成。

## 双链规则

- `[[笔记]]`：优先同目录；否则按全库同名匹配，唯一时跳转，多个时显示歧义并要求选择。
- `[[目录/笔记]]` 或 `[[/目录/笔记]]`：相对库根目录。
- `[[../笔记]]`：相对当前文件目录，不能越出库。
- 支持 `[[笔记|显示名称]]` 和 `[[笔记#标题]]`。
- 重命名不会自动重写其他文件中的链接；原始 Markdown 格式保持不变。

## 数据与恢复

Windows 应用数据在 `%LOCALAPPDATA%\Inkstone`。`recovery` 保存 JSON 恢复记录（包含正文），成功记录后缀为 `.saved`，失败 / 崩溃记录为 `.json`。界面提供每篇文件最新恢复草稿，恢复时新建笔记，不覆盖原文件。历史记录暂不自动清理。

删除会移动到笔记库的 `.inkstone-trash/<编号>/`，同目录有 `original-path.json`。当前可用资源管理器把文件移回原位置；尚无专门的图形回收站。

冲突时自动保存停止；“另存为副本”可保存当前编辑版本。普通保存错误也会停止自动重试，修复原因后按 Ctrl+S。IME 组词期间不会把预编辑拼音自动保存。异常退出仍可能丢失尚未写入恢复区的输入；慢速 IO 或组词期间可能超过一个自动保存周期。

## 实时预览边界

标题、`**粗体**`、单反引号代码、`[[双链]]` 在同一个可编辑原文中应用实时样式。光标进入语法范围显示标记，离开时隐藏；**隐藏保留标记宽度**，标题字号统一。复杂嵌套、多反引号、表格、图片仍有可靠源码路径和独立阅读预览。本版本没有实现 Obsidian 式消除标记占位的紧凑排版。

## 验证与限制

见 [测试记录](docs/TESTING.md)、[架构说明](docs/ARCHITECTURE.md)、[进度与待办](docs/PROGRESS.md)、[已知限制](docs/LIMITATIONS.md)。

核心安全测试通过不等于所有存储设备的断电保证；桌面输入、滚动、DPI、多图片、性能验收仍有未验证项。保留这些待办，不以已实现代码代替验收。
