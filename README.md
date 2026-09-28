# 砚台 / Inkstone

Rust + GPUI 本地 Markdown 笔记软件，当前处于**编辑器原型阶段**，尚未达到 MVP 验收条件。

## 构建与运行

Windows 需要 Rust stable（当前开发环境 1.98.1）、Visual Studio 2022 C++ Build Tools 和 Windows SDK。

```powershell
cargo run --locked
cargo test --locked --lib
cargo build --locked --release
.\target\release\inkstone.exe
```

这些是目标构建命令；实际执行结果见 [测试记录](docs/TESTING.md)。程序当前只编辑内存中的原型样本文档，关闭窗口会丢弃输入，请勿输入需要保留的笔记。

## 原型范围

- 原生 GPUI Component Editor；中文、emoji、软换行、剪贴板与撤销能力等待实际交互验证。
- 标题、粗体、单反引号代码、双链的字节范围样式。
- 光标离开语法范围隐藏标记，进入恢复；隐藏保留宽度。
- 源码切换与组件文档内查找。复杂语法保留源码。
- 后台样式扫描和解析版本校验。

## 文档

- [进度与待办](docs/PROGRESS.md)
- [架构与依赖决策](docs/ARCHITECTURE.md)
- [测试记录与人工验收](docs/TESTING.md)

笔记库、安全保存、阅读视图、全文搜索、双链导航等仍为必需待办。不能将当前原型当成完整笔记软件。
