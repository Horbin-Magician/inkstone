# 墨砚

Rust + GPUI 本地 Markdown 笔记应用，提供源码、实时预览和阅读三种模式，以及搜索、双链、标签、版本历史、备份和导出。整体目标尚未完成，当前进度和剩余问题见 [项目状态](docs/STATUS.md)。

## 启动

在仓库根目录打开示例笔记库：

```sh
cargo run --locked --bin inkstone -- demo-vault
```

将 `demo-vault` 替换为自己的笔记库路径；也可以不传参数，启动后选择文件夹。不要使用个人笔记做自动或原生验收。

macOS 打包后可双击 `target/debug/墨砚.app`；Windows release 构建后可运行 `target/release/inkstone.exe`。

## 项目结构


项目使用 Cargo workspace，依赖方向为 `inkstone-desktop → inkstone-core`：

- `crates/inkstone-core`：Markdown 解析与编辑规则、索引、搜索、文件存储、备份、导出及后台基准；不依赖 GPUI。
- `crates/inkstone-desktop`：GPUI 编辑器、工作区、原生窗口 / 菜单、图形适配及桌面集成测试；可执行文件仍名为 `inkstone`。

根目录统一维护依赖版本、`Cargo.lock`、release profile 和 vendor 补丁。正式图标由 `crates/inkstone-desktop/assets/` 管理，设计候选和提示词归入 `docs/design/branding/`；macOS 打包脚本与配置集中放在 `packaging/macos/`。

```text
crates/       核心逻辑、桌面界面与正式应用资源
packaging/    各平台打包脚本与配置
docs/        使用、状态、架构和验证文档（含 design/ 与 history/）
demo-vault/   示例笔记库
licenses/     第三方许可
vendor/       受控的第三方组件补丁
target/       本地构建产物与验收材料，已被 Git 忽略
```

## 构建与验证


Windows 已记录的构建环境：Windows 11、Rust 1.98.1、Visual Studio 2022 C++ Build Tools + Windows SDK。

```powershell
cargo build --release --locked --bin inkstone
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo run --release --locked -p inkstone-core --example benchmark -- 10000
```

只验证核心逻辑、无需构建 GPUI 时运行 `cargo test --locked -p inkstone-core`；桌面及其集成测试运行 `cargo test --locked -p inkstone-desktop`。CI 保留 macOS / Windows 全工作区检查，并增加 Ubuntu 核心独立检查。

两个 package 都是 workspace 默认成员，因此根目录原有的 `cargo test --locked`、`cargo build --locked --bin inkstone` 和 `cargo run --locked --bin inkstone` 仍可使用。

macOS 开发启动使用 `cargo run --locked --bin inkstone`，可执行文件内嵌应用名称配置与图标，系统菜单显示“墨砚”，启动时主动设置 Dock 图标。打包为可双击启动的应用：

```sh
bash packaging/macos/bundle.sh
open target/debug/墨砚.app
# 发布构建：bash packaging/macos/bundle.sh --release
```

打包脚本从 `crates/inkstone-desktop/assets/inkstone-icon.png` 生成多尺寸 macOS 图标，并使用本机临时签名，尚未配置发行签名和公证。当前界面固定为中文；将来切换界面语言时，需同步更新 `packaging/macos/Info.plist` 中的系统显示名称。Linux 尚未构建或验收。

## 文档入口

- [使用指南](docs/GUIDE.md)：功能、快捷键、双链和数据恢复。
- [项目状态](docs/STATUS.md)：当前进度、支持范围与验收缺口。
- [验证指南](docs/TESTING.md)：检查命令、原生验收原则和最近完整回归。
- [架构说明](docs/ARCHITECTURE.md)：模块职责、依赖方向和数据边界。
- [Markdown 支持矩阵](docs/MARKDOWN_SUPPORT.md)：语法、呈现与性能边界。
- [编辑器补丁](docs/EDITOR_PATCH.md)：第三方组件的本地改动与升级要求。

设计候选与提示词位于 `docs/design/branding/`；开发和验收旧记录位于 `docs/history/`。历史记录反映当时状态，当前待办以项目状态页为准。
