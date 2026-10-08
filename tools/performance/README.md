# 编辑性能验收 v1

本目录固定第 3 项的样本和测量条件。所有操作使用隔离应用数据与生成库，不能指向真实用户笔记。

## 样本

```sh
python3 tools/performance/corpus.py target/performance-v1
cargo build --release --locked -p inkstone-desktop
```

生成目录必须不存在；保留已编辑样本，在另一个新目录生成下一轮。`corpus-v1.json` 固定 UTF-8/LF、字节数、SHA-256 与场景。修改内容须提升版本，不能拿不同版本宣称前后改善。四个单文档场景以及 12 标签页场景都测量；长段落只有一段，不能拆段绕开问题。

## 隔离实例的启动与退出

macOS 可用准备工具创建全新、已临时签名的测试 bundle：

```sh
python3 tools/performance/prepare_macos.py target/native-ordinary-1 --scenario ordinary --mode live
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools/performance -p test_prepare_macos.py -v
```

`--scenario` 接受 corpus v1 的五个场景，`--mode` 为 live/source/reading；多标签按 manifest 顺序恢复 12 篇，首篇激活。每次选择不存在的新输出目录；已有目录或悬空链接会拒绝。工具只准备文件和验签，不启动应用、不测量、不操作窗口。通过原生应用启动工具打开输出的完整 bundle 路径，再确认库/模式/可见正文。重新启动同一 bundle 仍使用其隔离数据与最近库。`preparation.json` 记录已签名二进制散列、环境、场景及完整 corpus；构建提交、机器、PID、实际窗口和测量结果仍按下文另行记录。

显式指定生成库和独立 `XDG_DATA_HOME`，macOS 测试 bundle 也在 `Info.plist` 的 `LSEnvironment` 配置一个独立兜底数据目录，防止 UI 工具重新启动时读取用户默认最近库。兜底目录同样只能使用生成库；在签名前配置。绑定前先核对进程参数，绑定后再核对库/标签名称，发现非测试库立即停止测试操作。

正常退出后只通过已记录 PID 和启动包装进程退出码核对，不再对已退出实例调用 UI 状态查询或重新绑定；这类调用可能自动重启应用且丢失原启动参数。新一轮必须先由隔离启动器启动，再绑定运行中的实例。AX 正文并不单独证明画面已显示，采样前须确认实际画面；AX 焦点可能仍报告窗口，即使原生键入已进入编辑器。需要验证键盘输入时，可在生成笔记中键入唯一探针并逐次撤销至原文及“已保存”状态，记录实际操作与随后稳定期；不可在真实笔记中执行，也不可把经此预热的样本称为冷启动。该探针仅证明当时可输入，不能替代连续前台/焦点追踪；若需要缩放等干预，记录干预，并说明生命周期 RSS 包含干预过程。

## 优化前确定的预算

在同一台机器、1200×820 逻辑窗口、相同 DPI、默认字体/字号、release 构建下，五个场景都使用以下目标：原生输入到可见帧 p95 ≤ 50 ms、滚动可见帧间隔 p95 ≤ 33.4 ms；普通笔记打开到可编辑 ≤ 500 ms，其余单文档 ≤ 2 s，12 标签依次打开总计 ≤ 5 s。进程 RSS 峰值目标 ≤ 1 GiB，静置 30 秒后的连续 30 秒 CPU 均值 ≤ 单核 2%。这是事先设定的验收预算，不是已测得或已达到的数据；若要调整，必须记录理由和日期，不能用调整掩盖退化。

## 原生操作与记录

1. 记录 Git 提交、Rust 版本、机器/内存、系统、显示刷新率/DPI、输入法、窗口大小、字体、编辑模式和样本 manifest。关闭测试实例后再启动，冷启动与暖打开分开记录；不得声称 OS 文件缓存已清空。
2. 每场景先稳定 30 秒；依次测打开、原生键入（前/中/后各 20 次，5 次预热不计）、滚动（前/中/后各 20 次）和空闲。源码与实时预览分别记录；阅读模式测打开与滚动。多标签固定按 manifest 顺序打开全部 12 篇，在首/中/末标签重复输入与滚动。
3. 原生输入必须通过平台事件路径，记录从输入事件到显示帧的时间。使用带事件标记的原生帧追踪或高帧率视频；工具往返、截图耗时、直接修改编辑器状态都不能替代该指标。报告样本数量、p50/p95/max、测量分辨率和原始数据路径。打开应以内容可见且可接受输入为终点，不能以磁盘读取完成为终点。
4. RSS 峰值用平台 profiler 或进程退出时的资源统计获取；注明是否含 GPU 与子进程。下面的轻量采样给出采样 RSS 与累计 CPU 差分，可用于空闲 CPU 初测，不能独自证明峰值预算通过：

   ```sh
   python3 tools/performance/sample_process.py TEST_PID --settle-seconds 30 --seconds 30 > target/performance-idle.json
   ```

   明确填入隔离测试实例的 PID。工具不启动、不退出、不操作应用；PID 消失/复用或已退出但尚未回收即报错。`--settle-seconds` 默认为 0，正式测量显式传入 30；稳定期持续检查同一进程，结束后重新读取 CPU 基线，JSON 同时记录实际稳定时长与采样时长。`observed_peak_rss_kib` 只是采样期间最大 RSS（真实峰值下界），`interval_cpu_percent` 用累计进程 CPU 秒差值除以单调墙钟时间，100% 表示一核，精度受 `ps time` 输出限制；`median_ps_cpu_percent` 是系统定义的平均值，不能替代前者。
5. 每轮至少 3 次，逐轮保存原始数据，使用相同统计口径对比。同步、索引/公式首次缓存及草稿写入是否开启均须记录；不能只在优化后关闭后台工作。
6. 中文 IME 预编辑/提交、撤销/重做、跨 emoji/组合字符选区、源码坐标、模式切换及滚动锚点必须通过行为回归。性能达标不能代替正确性。

## 当前证据边界

已有 `frame_benchmark.rs` 为无头 GUI CPU 测量，样本和端到端范围与这里不同。已开始 release 原生资源初测，见 `docs/PERFORMANCE_BASELINE.md`；五场景完整基线、原生输入到显示帧与滚动数据仍待采集，不能据工具可运行判定性能验收完成。

## 多笔记历史目录基准

```sh
cargo run --release --locked -p inkstone-core --example history_catalog_benchmark -- target/history-catalog-new-run
```

输出目录必须不存在（包括符号链接），父目录需存在。工具只生成 1,000 篇笔记 × 10 版本的旧格式日志，不读取用户笔记，保留夹具与 `results.json` 供复查。正文包含中文、组合字符和 emoji。分别记录一次无元数据缓存的冷导入、三次暖目录扫描、一次只有逐记录缓存的单篇查询、三次单篇索引命中；每轮核对笔记数、版本数和总字节数，正文按需读取在计时之外验证。

这里的“冷”指应用元数据缓存为空，操作系统页缓存未清空。仅测后端墙钟时间，不包含原生历史窗口或输入延迟。比较改动时使用同机、同 release 参数和新输出目录，另记 Git 提交、编译器及二进制散列；不要将首次导入与暖缓存结果混合平均。

## 旧历史记录解析的内存取舍

此微基准独立于编辑器五场景验收。macOS 上执行：

```sh
cargo build --locked --release -p inkstone-core --example history_read_benchmark
python3 tools/performance/history_read.py
```

脚本在新的 `target/history-read-*` 目录生成一份旧格式日志，包含两段各约
32 MiB 的混合 Unicode/转义正文；不读取真实笔记。每种方式以独立子进程运行三轮，
交错顺序，保存原始 `/usr/bin/time -l` 输出、夹具/二进制散列和结果 JSON。
`whole` 重现原有整文件读取，`buffered` 使用 64 KiB 缓冲并保留完整 Recovery，
`metadata` 直接编译生产元数据解码模块，验证正文后只保留归属。三种模式循环轮换顺序，校验正文长度或归属输出，执行后复核夹具散列。
峰值 RSS 是子进程全程峰值，耗时覆盖文件读取和反序列化，不包括进程启动。
不强制清空系统文件缓存，不能将结果称为冷磁盘延迟。

2026-10-06，macOS 27.0.1 arm64 / Rust 1.97.0，夹具 77,276,882 字节，
SHA-256 `1540f197b3cf2727e5075ff80499196b6e5c46b75ab0b734bff456b45688583b`：

| 读取方式 | 三轮耗时中位数 | 峰值 RSS 中位数 |
| --- | ---: | ---: |
| 整文件 + from_slice | 118.868 ms | 177.297 MiB |
| BufReader + from_reader | 186.337 ms | 103.641 MiB |

脚本实测记录：`target/history-read-3y6tqom7`。缓冲方式 RSS 降约 41.5%，耗时增约
56.8%；保留其减少大日志额外内存副本的取舍。它仍完整解码正文，不是恒定总内存，
也不代表历史窗口打开、目录扫描、元数据写入或原生编辑性能已达标。

同日新增 metadata 模式的三方对照：`target/history-read-so950isu`，同一夹具散列。
完整缓冲方式 188.513 ms / 103.641 MiB，元数据方式 182.877 ms / 39.609 MiB
（三轮中位数）。具体逐轮数据、测量边界见 docs/PERFORMANCE_BASELINE.md。

采样器进程生命周期回归（不启动编辑器）：

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools/performance -p test_sample_process.py -v
```

## 同步扫描与文件处理成本

macOS / Python 3.11+：

```sh
cargo build --release --locked -p inkstone-core --example sync_scan_benchmark
python3 tools/performance/sync_scan.py
```

脚本在新的 `target/sync-scan-*` 目录生成两种固定夹具：一万个 1 KiB 文件、九个 64 MiB 文件（576 MiB），逐文件记录 SHA-256。每种夹具以独立子进程运行三轮，每轮使用全新的恢复目录；不要把示例指向用户笔记或已有恢复目录。示例调用实际同步入口，在 Scanning 完成后、读取远端清单之前取消，任何远端方法被调用都会失败，不上传或发布。

记录目录枚举、文件处理、清单校验与总扫描耗时；文件处理包含路径检查、打开、读取、SHA-256 和结果插入，不能称为纯哈希计算时间。测量不含同步锁/缓存初始化、网络、验证复扫及应用阶段；进程峰值 RSS 则包含示例整个生命周期。脚本在计量进程外生成夹具并最终重新校验所有正文。文件系统缓存不清空，第一次运行也不是冷磁盘延迟。原始输出、time 日志、二进制/源码散列及各轮统计保存在生成目录。

## 原生窗口活动诊断

准备隔离 bundle 时可增加 `--trace-activity`。应用仅在显式设置 `INKSTONE_TRACE_ACTIVITY` 时创建指定日志，文件必须不存在，父目录必须已有；准备工具将其限制在本次输出目录的 `activity.jsonl`，不会提前创建日志。再次启动同一实例时旧日志保留，本次记录关闭；正式新一轮应准备新目录。

现有两秒工作区定时器在 tick 前记录 PID、Unix 毫秒时间、启动后单调毫秒、窗口激活、当前编辑器焦点及加载状态，最多 1,800 条，写失败即停止。不记录笔记路径、正文、选区或输入内容。未开启时不创建文件，不为诊断增加定时器。日志写入有少量开销，前后对照必须保持相同设置；它是间隔采样，不能证明两个采样点之间没有切换焦点，不能替代显示帧追踪。

sample_process.py 的 `sample_start_unix_ms` / `sample_end_unix_ms` 可用于筛选活动日志中正式 CPU 采样区间内的记录；CPU 耗时仍使用单调时钟。缺失记录、仍在加载或窗口未激活时应单独报告，不能只看低 CPU 宣称前台预算通过。

活动日志还记录当前编辑器的累计 `editor_renders` 和 `input_notifications`；无活动
编辑器时为 null，切换编辑器后计数可能重置。仅启用活动诊断时累加，不增加定时器。
正式区间需同时检查计数增量：前台/焦点为 true 但没有持续重绘时，低 CPU 不足以
代表闪烁光标的空闲成本。渲染计数是 CPU 侧视图组装次数，不是显示帧或 GPU 提交数，
不能替代真实画面、原生输入与帧跟踪。不同诊断字段版本的开销需在对照中说明。

## 全面性能工作恢复（2026-10-07）

当前计划仅对 macOS 做性能验收，保留 Windows 正确性 CI。所有旧结果保持原有
边界；重新以当前提交测量，不将原生帧数据缺失记为通过。

```sh
cargo build --release --locked -p inkstone-core --example benchmark
python3 tools/performance/backend.py
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools/performance -p test_backend.py -v
```

`backend.py` 对 1,000 / 10,000 篇固定内容的生成库各启动三个独立进程，保存版本化
JSON、每轮 stdout/stderr、样本逐文件散列、提交/工作树差异、硬件、编译器和二进制散列。
它不接收用户库路径。计时来自原有 benchmark 的后端阶段；RSS 是整个子进程峰值，
包含夹具生成、所有阶段和析构，不能当作应用峰值或单阶段内存。夹具核验在计量进程外。
不清 OS 缓存，不与构建/其他测试并行运行。`--binary` 可选择保留的对照二进制；提交号
指运行时工作树，比较旧二进制时另行记录其构建提交。原始数据只保留在忽略目录。

## 可选阶段计时（schema v1）

`INKSTONE_TRACE_PERFORMANCE` 指向父目录已存在的新 JSONL 文件；未设置时不创建文件，
也不为 span 读取时钟。已有文件（含符号链接）不会被覆盖，打开失败时诊断关闭。
每进程最多 10,000 条，写失败停止记录。日志只含固定 stage、PID、序号、起点 Unix
毫秒、单调起点微秒及耗时微秒，不含路径、查询、正文。跨度为墙钟时间、允许嵌套，
不能相加作为总 CPU，不能当作输入到呈现帧时间。

```sh
python3 tools/performance/prepare_macos.py target/new-diagnostic-run --scenario large-document --mode live --trace-activity --trace-performance
```

准备器只设置隔离 bundle 环境，不启动程序。阶段包括完整/局部语法、阅读投影、片段、
展示更新、编辑器视图组装、索引加载、两类搜索、文件刷新、保存事务和同步扫描/事务。
`editor_view_composition` 只量化元素组装，不包含 GPUI 后续布局、绘制或 GPU 提交；
后者仍结合现有无头 frame benchmark 和平台 profiler，不以本日志替代原生帧验收。
日志同步落盘有诊断成本；正式预算采样默认关闭，前后对照必须保持同一设置。

## 阅读与片段准备基准

```sh
cargo build --release --locked -p inkstone-core --example projection_benchmark
# GENERATED_FILE 必须来自 corpus.py 新生成的隔离库。
target/release/examples/projection_benchmark GENERATED_FILE
```

语法快照在计时前创建；每进程 5 次预热、20 次正式样本，输出总 p50/p95 与 p50 对应
的阅读投影耗时，并核对每轮输出散列、任务数、片段数。输出源/结果散列用于跨版本
一致性检查；源码位置、任务行为和链接语义仍须单独回归。没有包含最初语法解析、
图形栅格化、GPUI 布局、磁盘打开或原生帧延迟。比较版本时先构建并保留二进制和
散列，再停止构建，至少三对独立进程交错顺序运行同一 corpus，保存原始 stdout。

## 单个长段落的 GUI CPU 对照

```sh
cargo test --locked --release -p inkstone-desktop long_paragraph_frame_performance -- --ignored --nocapture --test-threads=1
```

无头基准内置 corpus v1 的 `long-paragraph.md`，运行前校验 manifest 散列，不拆段。
源码与实时预览分别在段首、中、末测强制重绘、24 像素往返滚动、交替插字/换行，
每组 5 次预热、20 次正式样本，JSON 行保留全部耗时及 p50/p95/max。每次编辑后
撤销并断言正文完全恢复；撤销和重新定位不计入编辑样本。计时包含同步操作、
排空可运行任务及绘制，窗口为 1200×820；不含完整工作区、原生输入事件、GPU
呈现或实际闪烁定时器，不能据此宣布原生延迟或空闲 CPU 百分比达标。
不同构建配置不可混比；先保留前后测试可执行文件，停止编译后以独立进程交错运行。

可见范围查询的独立定位基准（先通过 vendor 回归入口准备当前暂存源码）：

```sh
python3 tools/vendor-regression/run.py
cargo test --locked --manifest-path target/vendor-regression/Cargo.toml --target-dir target -p gpui-base --lib long_paragraph_range_performance -- --ignored --nocapture --test-threads=1
```

它对同一 corpus v1 长段落的已准备映射交错调用逐显示行扫描的旧算法和当前查询，
每组 1,000 次、三组，并核对所有结果。这里的旧算法是在测试中重建的原实现，
不是旧版应用二进制；默认 debug 配置，排除初始整形、编辑、绘制和原生交互，
不能将其改善倍数作为整段输入或滚动的改善倍数。
