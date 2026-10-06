# 编辑性能验收 v1

本目录固定第 3 项的样本和测量条件。所有操作使用隔离应用数据与生成库，不能指向真实用户笔记。

## 样本

```sh
python3 tools/performance/corpus.py target/performance-v1
cargo build --release --locked -p inkstone-desktop
```

生成目录必须不存在；保留已编辑样本，在另一个新目录生成下一轮。`corpus-v1.json` 固定 UTF-8/LF、字节数、SHA-256 与场景。修改内容须提升版本，不能拿不同版本宣称前后改善。四个单文档场景以及 12 标签页场景都测量；长段落只有一段，不能拆段绕开问题。

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
