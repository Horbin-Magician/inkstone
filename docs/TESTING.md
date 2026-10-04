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
- 公式文档准备基准：`cargo run --locked -p inkstone-core --example formula_benchmark`（同机、相同构建模式比较；不包含公式图像生成、界面布局或真实输入延迟）。
- 公式界面 CPU 基准：`cargo test --locked -p inkstone-desktop formula_frame_performance -- --ignored --nocapture`（无头窗口 1200×820，100 / 1000 条不同分式，25 次编辑及滚动、剔除前 5 次；打开阶段含后台任务及 12 次绘制，不是真实首帧或输入延迟）。
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

## 公式密集文档准备优化（2026-10-04）

- 公式和 Mermaid 片段从现有语法树一次建立索引，直接保留原文与源位置，避免每条公式重新扫描整篇文档；表格、提示块和脚注继续沿用原有展开逻辑。
- 同机 macOS debug 后台基准，每种样本运行三次取总耗时中位数：100 / 500 / 1000 条独立公式由 21.895 / 404.605 / 1517.589 ms 降至 10.573 / 33.081 / 79.868 ms。其中 1000 条公式的片段准备由 1461.429 ms 降至 27.471 ms。此数据不是原生窗口打开时间或输入延迟。
- 核心测试 181 项通过、1 项原有手动基准忽略，覆盖公式原文和源位置等价性、图表、引用容器、脚注及嵌套公式归属；核心 Clippy、格式与差异检查通过。使用生成样本，没有修改用户笔记。
- 公式测量移到共享缓存锁之外，避免后台计算期间阻塞界面查询图片缓存；并发完成时复用已有结果，保留已经生成的 SVG。新增八线程并发缓存测试通过；桌面 253 项单元测试及 6 项集成测试通过，1 项原有手动基准忽略，桌面 Clippy 通过。可见区域渲染、公式基线、编辑后复用及输入法测试均通过；未进行原生窗口视觉或真实输入延迟验收。

### 行高投影优化（2026-10-04）

- 行高投影改为一次建立行号索引，消除每条公式扫描已有样式并重复换算 Rope 位置的平方级开销。同一行多个公式仍取最大高度，保留标题字号和既有样式顺序。
- 同机 macOS debug 公式界面 CPU 基准，1000 条公式：打开及稳定绘制阶段由 4790.198 ms 降至 2002.544 ms，编辑加绘制 p50 由 1556.715 ms 降至 95.655 ms，p95 由 1588.672 ms 降至 97.874 ms。滚动 p50 为 49.449 → 51.357 ms，p95 为 1591.665 → 140.615 ms；正常滚动没有明显改善，较慢帧中的重新投影开销降低。此结果独立于上一轮核心后台基准，不可混作同一指标。
- 新增行高合并回归覆盖同一行多个公式、已有标题字号、乱序样式、CRLF 和多行公式。桌面 253 项单元测试及 7 项集成测试通过，2 项手动基准默认忽略；桌面 Clippy、工作区格式和修改文件格式检查通过。没有修改用户笔记，未做原生窗口输入延迟验收。

### 公式位置查询优化（2026-10-04）

- 显示对象新增 ID 索引，安装、清空及输入法位置调整时同步更新，保持重复 ID 的首次匹配语义。查询时提前排除布局范围外的锚点，避免逐个屏外公式遍历可见行。
- 相同公式界面 CPU 基准，1000 条公式在行高优化基础上进一步达到：打开及稳定绘制 1785.641 ms，编辑加绘制 p50 / p95 为 83.723 / 86.909 ms，滚动 p50 / p95 为 23.463 / 98.871 ms。相较本轮两项优化前，编辑中位数约快 18.6 倍，滚动中位数约快 2.1 倍；仍为同机 debug 无头测量。
- 新增滚动位置与原始范围查询等价性测试，覆盖逆序 ID、重复 ID、对象替换及清空；已有输入法、软换行和滚动锚点回归通过。桌面 253 项单元测试及 8 项集成测试通过、2 项手动基准默认忽略；桌面 Clippy、格式与差异检查通过。

## WebDAV 云同步（2026-10-04）

- 核心新增 8 项测试：双设备笔记/二进制附件往返、中文及 URL 特殊字符文件名、修改/重命名/删除、冲突收敛、删除与修改冲突、上传失败重试、损坏内容、路径穿越/大小写/符号链接、端点隔离；本地 HTTP 服务检查认证、条件 PUT、并发拒绝、重定向与弱 ETag 拒绝。
- 桌面新增本地 WebDAV 服务集成测试：等待未保存草稿、上传最新正文、下载后更新文件列表、配置保存但密码不落盘、清除会话密码、保存错误阻止同步。
- `cargo test --locked --workspace`：435 项通过、2 项原有手动基准忽略；`cargo clippy --locked --workspace --all-targets -- -D warnings`、`cargo fmt --all --check` 和 `git diff --check` 通过。
- 使用临时笔记库，没有操作用户笔记。尚未使用真实服务商账号，也未完成新增面板的原生视觉验收或 Windows/Linux 实机验证。
- 后续恢复保护补充：下载替换保留 Unix 原权限；替换与删除生成记录原路径、备份文件名及 SHA-256 的恢复侧文件，超长文件名在提交前拒绝。同步专项 9 项测试全部通过，核心 Clippy、格式与差异检查通过；此补充之后未重复全工作区测试。
