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
macOS 需要 Xcode Command Line Tools，按本机 Rust host 架构输出 DMG 和 app.tar.gz；应用使用临时签名。
macOS Intel 与 Apple Silicon 使用各自 runner 构建。
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
