# 墨砚

墨砚是一款用 Rust 和 GPUI 构建的中文桌面笔记应用。打开一个本地文件夹即可作为笔记库，笔记以 Markdown 文件保存。你可以在源码、实时预览和阅读模式间切换，也可以分屏对照同一篇笔记。

## 安装与首次使用

Windows x64 可选择安装器或便携 ZIP，Apple Silicon Mac 可下载 DMG。发行包见 [GitHub Releases](https://github.com/Horbin-Magician/inkstone/releases)，版本改动见 [0.5.1 发布说明](docs/releases/0.5.1.md)。

1. 启动墨砚，选择已有笔记文件夹，或创建一个新库。
2. 在左侧文件列表打开一篇笔记，或按 `Ctrl+N`（macOS 为 `Cmd+N`）新建笔记。
3. 在源码或实时预览模式中右键正文，可快速插入表格、代码块、公式块，或从子菜单设置标题、列表和文本格式。光标在表格内时还可编辑行列与对齐。也可在行首输入 `/table` 或 `/math` 后按 Enter；只输入 `/` 浏览全部块选项。
4. 按 `Ctrl+S`（macOS 为 `Cmd+S`）保存到原文件。

## 整理和使用笔记

- **查找与关联**　按文件名打开笔记，或用全文搜索查找正文。输入 `[[` 可补全双链，通过反向链接查看哪些笔记引用了当前页。标签用于跨目录分类，大纲可跳转到当前笔记的标题。
- **同步与恢复**　WebDAV 可在设备间同步笔记库。需要找回内容时，可查看单篇笔记的版本历史，或从整库备份恢复。同步、备份和导出的操作见 [使用指南](docs/GUIDE.md)。
- **Agent 笔记助理**　在设置中配置模型服务后，可从右侧栏就当前笔记或选区提问，点击回答中的来源回到原文。也可以指定文件夹或整个笔记库，让助理整理内容。修改会先显示差异，接受后才会应用。详见 [Agent 指南](docs/AGENT.md)。

## 从源码运行

仓库通过 `rust-toolchain.toml` 固定 Rust 1.97.0。macOS 需要 Xcode Command Line Tools，Windows 需要 Visual Studio 2022 C++ Build Tools 和 Windows SDK。

在仓库根目录运行以下命令，再从文件列表打开《欢迎》体验示例笔记库。

```sh
cargo run --locked --bin inkstone -- demo-vault
```

可将 `demo-vault` 替换为自己的笔记库路径，或省略 `-- demo-vault`，启动后选择文件夹。自动测试和原生验收请使用示例库或临时库。

## 开发与验证

Cargo workspace 包含三个 crate。

| Crate | 职责 |
| --- | --- |
| [inkstone-core](crates/inkstone-core) | Markdown 解析与编辑规则、文件存储、索引、搜索、备份和导出 |
| [inkstone-agent](crates/inkstone-agent) | 模型协议、授权范围内的笔记检索、会话和修改提案，依赖 core |
| [inkstone-desktop](crates/inkstone-desktop) | GPUI 编辑器、工作区、原生窗口和菜单，集成 core 与 agent，生成可执行文件 `inkstone` |

在仓库根目录构建和检查整个工作区：

```sh
cargo build --release --locked --bin inkstone
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
```

只验证核心逻辑可运行 `cargo test --locked -p inkstone-core`。运行万篇笔记的核心基准可用 `cargo run --release --locked -p inkstone-core --example benchmark -- 10000`。桌面 CI 在 macOS 和 Windows 上运行工作区检查。原生验收范围见 [验证指南](docs/TESTING.md)。

macOS 本地开发可运行以下命令生成并打开应用包。发行构建、签名、公证与校验步骤见 [原生打包指南](packaging/README.md)。

```sh
bash packaging/macos/bundle.sh
open target/debug/墨砚.app
```

## 进一步阅读

- [项目状态](docs/STATUS.md)记录已实现的功能、平台支持范围和待验收事项。
- [Markdown 支持矩阵](docs/MARKDOWN_SUPPORT.md)列出各类语法的呈现方式与限制。
- [架构说明](docs/ARCHITECTURE.md)介绍模块职责、依赖方向和数据边界。
- [编辑器补丁](docs/EDITOR_PATCH.md)说明第三方组件的本地改动与升级要求。
