# 验证指南

从仓库根目录执行：

```sh
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
```

仅改核心时可以先运行 `cargo test --locked -p inkstone-core`；桌面及集成测试使用 `cargo test --locked -p inkstone-desktop`。按改动范围选择必要检查，不将编译通过视为原生验收通过。

## macOS 打包检查

```sh
bash -n packaging/macos/bundle.sh
bash packaging/macos/bundle.sh
codesign --verify --deep --strict target/debug/墨砚.app
plutil -lint target/debug/墨砚.app/Contents/Info.plist
```

发布构建使用 `bash packaging/macos/bundle.sh --release`，对应产物在 `target/release/`。图标由 desktop 的正式资源生成；不需要设计候选图或生成工具。

## 原生验收与性能

- 使用独立测试笔记库，不操作用户笔记；记录平台、构建模式、输入法、DPI 和样本条件。
- 编辑行为检查保存后正文、撤销/重做、选区、组词及视图切换；渲染行为分别检查源码、阅读和实时预览。
- 无头基准、自动测试、真实窗口视觉和输入延迟分别记录，不能互相替代。
- 核心后台基准：`cargo run --release --locked -p inkstone-core --example benchmark -- 10000`。
- `target/` 被 Git 忽略，但当前还包含验收库、备份和导出结果。清理前先保留需要的验收材料，再清理可重新生成的构建产物。

## 最近一次完整回归（2026-10-04，workspace 拆分）

以下是此前已记录的结果，不代表每次文档修改重新执行了全量检查。

- 原 library 与 binary 分别迁入 `crates/inkstone-core` 和 `crates/inkstone-desktop`；可执行文件名保持 `inkstone`。103 个原源码、示例与测试文件均有唯一迁移目标，第三方依赖版本未变化。
- 独立运行 `cargo test --locked -p inkstone-core`：170 项通过、1 项手动基准忽略；`cargo test --locked -p inkstone-desktop`：250 项桌面测试及 6 项集成测试通过、1 项手动基准忽略，总计 426 项通过。
- `cargo tree --locked --offline -p inkstone-core --edges normal,build,dev` 确认核心的运行、构建和测试依赖均不含 GPUI；workspace 元数据确认两个默认成员，根目录普通测试命令仍覆盖两包。
- 全 workspace 与独立 core 的 `cargo clippy --locked … --all-targets -- -D warnings` 均通过；`cargo fmt --all --check` 与差异检查通过。根目录 `cargo run --locked --example benchmark -- 100` 完成，作为迁移后入口验证，不用于声明性能提升。
- `bash packaging/macos/bundle.sh` 构建成功；`codesign --verify --deep --strict` 与 Info.plist 校验通过，二进制保留 `__TEXT,__info_plist` 段。未启动原生窗口，未修改用户笔记。
- CI 明确执行 workspace 检查，增加 Ubuntu 独立核心任务；YAML 与脚本语法已检查，远程 CI 和 Windows / Linux 实机结果尚未验证。

## 目录整理检查（2026-10-04）

打包脚本迁入 `packaging/macos/`；正式图标归入 `crates/inkstone-desktop/assets/`。脚本语法、workspace 格式检查、macOS debug 构建和打包、签名校验通过。文档整理检查覆盖 11 份文档的本地链接、四份历史记录正文完整性及示例笔记无改动。此次只调整路径和文档，没有启动原生窗口验收，也没有重新运行全量行为测试。

逐步开发与原生验收证据保存在 [历史测试记录](history/TESTING.md)；待完成项统一维护在 [项目状态](STATUS.md)。

## WebDAV 云同步（2026-10-04）

- 核心新增 8 项测试：双设备笔记/二进制附件往返、中文及 URL 特殊字符文件名、修改/重命名/删除、冲突收敛、删除与修改冲突、上传失败重试、损坏内容、路径穿越/大小写/符号链接、端点隔离；本地 HTTP 服务检查认证、条件 PUT、并发拒绝、重定向与弱 ETag 拒绝。
- 桌面新增本地 WebDAV 服务集成测试：等待未保存草稿、上传最新正文、下载后更新文件列表、配置保存但密码不落盘、清除会话密码、保存错误阻止同步。
- `cargo test --locked --workspace`：435 项通过、2 项原有手动基准忽略；`cargo clippy --locked --workspace --all-targets -- -D warnings`、`cargo fmt --all --check` 和 `git diff --check` 通过。
- 使用临时笔记库，没有操作用户笔记。尚未使用真实服务商账号，也未完成新增面板的原生视觉验收或 Windows/Linux 实机验证。
- 后续恢复保护补充：下载替换保留 Unix 原权限；替换与删除生成记录原路径、备份文件名及 SHA-256 的恢复侧文件，超长文件名在提交前拒绝。同步专项 9 项测试全部通过，核心 Clippy、格式与差异检查通过；此补充之后未重复全工作区测试。
