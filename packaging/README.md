# 原生构建与发布

参考 Rotor 的原生发布流程：锁定工具链和依赖、构建后打包、清单校验、版本标签校验、全平台成功后发布。
版本源为根 Cargo workspace；Python 3.10+，无需 pip 包。

```powershell
python packaging/package.py target/package-local
python -m unittest discover -s packaging -p "test_*.py"
```

输出目录必须不存在。脚本每次先构建 release，再复制当前产物，禁止复用手工指定的旧二进制。
Windows 需要 MSVC C++ Build Tools、Windows SDK 和 NSIS 3（可通过 `NSIS_MAKENSIS` 指定 makensis.exe）。
产物为 x64 便携 ZIP、当前用户安装器 EXE；使用静态 C 运行库，无需另装 Visual C++ 运行库。安装器无需管理员权限，卸载保留用户创建的文件及应用数据。
Windows 程序内嵌多尺寸应用图标；安装器和卸载器使用同一图标。安装时创建当前用户的桌面与开始菜单快捷方式，卸载时移除这两个快捷方式。
更新图标原稿后，安装 Pillow 并运行 `python packaging/windows/generate_icon.py`，重新生成并提交 `crates/inkstone-desktop/assets/inkstone.ico`。生成器去除原稿为 macOS Dock 预留的透明边距，保持图案比例，并输出 16–256 像素的 Windows 图标（含常见缩放比例需要的 20、40、96 像素）。正常构建使用已提交的 ICO，无需 Pillow。
macOS 仅提供 Apple Silicon 构建，需要 Xcode Command Line Tools，输出 DMG 和 app.tar.gz；应用使用临时签名。
CI 使用 Apple Silicon runner 构建 macOS 发行包。
DMG 按源文件逻辑大小额外预留 20% 和 64 MiB 的文件系统空间，避免自动估算容量不足。
包内包含使用指南、许可和 `resources.json`，包外 `.sha256` 校验下载完整性；校验和不等同于数字签名。
当前没有 Windows 发行签名、Apple Developer ID 公证或自动更新功能。

## 发布

1. 修改根 `Cargo.toml` 版本并运行 `cargo check --workspace` 更新锁文件，提交版本修改。
2. 写入并提交 `docs/releases/<版本>.md`，完成工作区测试和打包检查。
3. `python packaging/release.py <版本> --dry-run` 检查版本、说明、干净工作区和标签。
4. `python packaging/release.py <版本>` 创建标签并原子推送当前分支和标签。

`publish.yml` 校验标签与 Cargo 版本一致，执行 Windows / macOS 测试并打包。
所有平台成功后先上传附件至草稿，再公开 GitHub Release。失败时不会公开不完整版本。
已存在的正式 Release 不覆盖；草稿可通过手动运行工作流重试。手动运行必须选择对应版本标签。
日常 `package.yml` 支持手动构建候选产物，不发布 Release。
CI 将待发布文件写入 runner 临时目录，与 Cargo 依赖缓存隔离；分支检查会取消同一分支的旧任务，版本标签只触发发布工作流。

## 安装界面资源

Windows 使用纸白背景、墨色文字与现有应用图标，支持 DPI 缩放；安装完成页可直接启动应用。引导文案提供中英文版本，安装与卸载共用品牌页眉。
`packaging/windows/assets/` 中的 BMP 是已提交的构建输入。修改视觉资源时，安装 Pillow 11.3.0 并运行 `python packaging/windows/generate_installer_art.py`；正常打包无需 Pillow。

macOS DMG 使用 640 × 400 的暖白窗口，左右并列显示墨砚和 Applications，窗口标题提示拖放安装；指南和许可收纳在下方的“使用指南与许可”文件夹。DMG 根目录使用隐藏的 `.resources.json` 记录调整布局后的文件路径；`app.tar.gz` 仍保留原来的目录结构和 `resources.json`。
Finder 布局来自已提交的 `packaging/macos/finder-layout.dsstore`，无需在 CI 中启动 Finder 或授权 AppleScript。调整布局时，安装 ds-store 1.3.1 并运行 `python packaging/macos/generate_layout.py`。纯色背景不依赖本机图片路径或卷别名。
