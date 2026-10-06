# 编辑性能基线记录

固定样本、事先预算和测量流程见 [规程](../tools/performance/README.md)。当前仅有初测，尚未完成第 3 项性能验收。

## 2026-10-05 原生 release 资源初测

- 应用代码：`a33648b`，Rust 1.97.0，`cargo build --release --locked -p inkstone-desktop`。二进制 SHA-256：`16e0be90c676a214854b92832b8b2d238c5644fcd3b34fee33fba67cef4a5fee`。此次只修改 Python 测量工具，未修改应用代码。
- 机器：Apple M4，24 GiB，macOS 27.0.1（26A434），arm64。窗口 1200×820 逻辑像素，截图 2400×1640；默认 16 字号、深色、实时预览。显示刷新率未记录，未操作输入法。
- 实例：独立 bundle `app.inkstone.performance-test`，应用数据与库分别位于 `target/performance-native/data`、`target/performance-v1`。库含 corpus v1 全部 16 文件，只恢复 `ordinary.md`；索引首次构建，缓存 hits=0/misses=16。已通过原生截图确认内容显示。同步 URL 为空；草稿功能保持默认，但没有编辑操作。
- 本轮单次进程从启动至正常退出 56.86 秒；并非完整的 3 轮基线。原测试窗口外的用户应用未退出或修改。

| 指标 | 初测 | 解释 |
| --- | ---: | --- |
| main → loaded_frame_boundary | 882.407 ms | 含框架启动/索引；帧调度边界，不是笔记打开到可编辑或屏幕呈现时间 |
| load_requested → ui_loaded | 224.052 ms | 后台加载与 UI 应用阶段，仅辅助定位 |
| 进程生命周期最大 RSS | 293,634,048 B（280.031 MiB） | macOS `/usr/bin/time -l` 退出统计，非采样峰值，不含独立 GPU/辅助进程 |
| 系统报告 peak memory footprint | 236,422,320 B | 与 RSS 口径不同，不能混用 |
| 30.011 秒区间 CPU | 单核 2.133% | 累计 CPU 秒差分/单调墙钟；`ps time` 精度约 0.01 秒 |
| 采样 RSS 最大值 | 284,448 KiB | 采样期下界，不替代生命周期峰值 |
| 原生输入到显示 / 滚动帧间隔 | 未取得 | 不使用工具往返或截图耗时代替 |

采样在启动后约二十余秒开始，未严格完成规程要求的 30 秒稳定期；因此 CPU 数值不用于预算通过/失败结论。窗口前台状态和显示刷新率也需在正式轮次补记。普通场景以外的四个场景、三轮重复与模式矩阵尚未采集。

本机 `xcrun --find xctrace` 返回未安装。现有启动 trace 只有 CPU/调度阶段；仍需可验证的原生事件到显示帧测量途径。此限制不改变原生延迟验收要求。

原始日志保留在忽略目录：`target/performance-release-build.log`、`target/performance-native/startup-resource.log`、`target/performance-native/ordinary-idle.json`。通过 Command-Q 正常退出，执行会话退出码 0，进程检查确认隔离实例已结束。所有生成 Markdown 的哈希仍匹配 corpus v1；无用户笔记进入代码提交。


## 块级解析 CPU 对照（2026-10-05，非原生延迟）

同机 Apple M4、Rust 1.97.0 release，corpus v1，5 次预热后 20 次编辑；同一输入分别完整解析和局部更新，每次在计时外比较完整 AST。普通样本在“**重点**”内插入中文，大文档在“段落 1000”标题内插入中文。编译及全量测试结束后独立重测，以下为单轮结果，不混入上面的原生资源初测：

| 样本 | 完整解析 p50 / p95 | 局部更新 p50 / p95 |
| --- | --- | --- |
| ordinary.md，3,945 B | 0.518 / 0.563 ms | 0.028 / 0.030 ms |
| large-document.md，1,720,682 B | 112.748 / 113.863 ms | 2.608 / 2.656 ms |

复现：`cargo run --release --locked -p inkstone-core --example block_benchmark -- target/performance-v1/ordinary.md 重`，第二个场景将参数换成 `target/performance-v1/large-document.md '段落 1000'`。最终日志为 `target/block-benchmark-ordinary-final.log` 与 `target/block-benchmark-large-final.log`；较早带并行构建干扰的日志不纳入表格。

此测量不包含原生输入、展示投影、布局或显示提交；不能用来宣布 50 ms 输入预算达标。局部更新仍克隆 AST 和更新后续坐标；跨行、嵌套容器和全局引用等仍回退完整解析。

## 展示对象匹配实验（2026-10-05，未保留实现）

在 `9ea9ab0` 基础上尝试哈希分组复用旧对象、二分查找语法候选。运行原有 `formula_frame_performance`（debug 无头，1200×820，5 次预热后 20 次采样）。前后测量期间没有并行执行编译/其他测试，但未控制机器其他进程负载；仅一次对照，不能确认差异原因。

| 公式数量 | 编辑 p50 / p95，改前 → 实验 | 滚动 p50 / p95，改前 → 实验 |
| --- | --- | --- |
| 100 | 8.682 / 8.914 → 9.369 / 9.554 ms | 4.112 / 11.471 → 4.457 / 12.407 ms |
| 1000 | 77.602 / 78.888 → 79.081 / 80.027 ms | 21.346 / 87.912 → 22.159 / 92.149 ms |

开库稳定绘制阶段：100 公式 281.968 → 319.007 ms；1000 公式 1654.390 → 1752.176 ms。匹配理论复杂度改善没有体现为本轮整体耗时收益，故撤回生产实现，保留重复公式视图实体/几何复用回归。后续需定位解析、投影准备和绘制的主要成本，而非据此宣称该匹配算法必然更快或更慢。

原始日志：`target/object-match-before.log`、`target/object-match-final-benchmark.log`；最初的候选哈希索引中间版本日志为 `target/object-match-after.log`。以上都不是原生输入/滚动延迟。


## 普通笔记实时预览：三轮空闲 CPU / RSS（2026-10-06）

- 应用代码 `87af27b514fd8aca331023bb6f53c7c6a620259e`，Rust 1.97.0，locked release；隔离 bundle 二进制 SHA-256 `7a27092670efa7f6b213319990e279eb9a910be5989b5a48ce7cd79e5a6d0755`。Apple M4 / 24 GiB / macOS 27.0.1（26A434）。每轮使用独立 XDG_DATA_HOME 和重新生成的完整 corpus v1 库，显式恢复 ordinary.md，实时预览、默认 16 字号、深色，两侧栏打开；首轮截图为 2400×1640（1200×820 逻辑窗口），后两轮相同启动设置且通过 AX 核对正文/模式。
- 本轮为三个独立进程，应用数据与索引均不复用；没有清空 OS 文件缓存。同步 URL 为空、定时备份未启用，草稿功能保持默认但没有输入。每轮确认正文加载后才启动采样，采样器完整观察同一 PID 30 秒再测 30 秒，CPU 基线在稳定期结束后读取；采样期间不运行构建/测试、不编辑或滚动。用户其他应用未退出；未持续记录前台应用、显示刷新率或输入法，因此仍不是完整受控模式/显示矩阵。

| 轮次 | 实际稳定期 / 采样期 | 区间 CPU（单核） | 生命周期峰值 RSS | 采样 RSS 最大值 |
| --- | --- | ---: | ---: | ---: |
| 1 | 30.010 / 30.009 s | 1.100% | 286.453 MiB | 261,392 KiB |
| 2 | 30.007 / 30.010 s | 1.200% | 287.250 MiB | 269,616 KiB |
| 3 | 30.005 / 30.011 s | 1.899% | 285.969 MiB | 261,168 KiB |

三轮 CPU 中位数为 1.200%，峰值 RSS 中位数为 286.453 MiB；这组观测低于空闲 2% / RSS 1 GiB 预算，但仅覆盖普通笔记实时预览静置场景，不能判定整个第 3 项通过。RSS 来自 `/usr/bin/time -l` 正常退出统计，不含独立 GPU/辅助进程；采样最大值仅作对照。另保存系统 peak footprint 原值，其口径不同，不与 RSS 混用。没有测得原生输入到显示、滚动或打开到可编辑时间；其余四场景与模式仍需补齐。

原始证据：`target/performance-idle-0mp287n8/run.json`、`results.json`，每轮子目录的 `idle.json`、`resources.log`、`stdout.log`、PID 与退出码；构建日志 `target/performance-idle-release-build.log`。三轮均 Cmd+Q 正常退出、执行会话退出码 0，进程检查确认结束；每轮全部十六篇 Markdown 的 SHA-256 均匹配 corpus v1。未读取或改写真实用户笔记。
