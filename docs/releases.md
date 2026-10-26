# 安装、升级与发行

CLI 源码构建方法见 [README](../README.md#安装与构建)。Python 3.11 及以上仅用于开发和打包，已构建的 CLI 无需 Python 或 Rust；读取 Git 知识包仍需要 Git。

## 下载与安装

已发布版本从 [GitHub Releases](https://github.com/coolplayagent/okf-cli/releases) 下载。工作流产物或 Release 草稿不等同于正式发布。

选择与系统、CPU 和运行环境相符的 `okf-cli-VERSION-TARGET.zip` 及同名 `.zip.sha256`。`TARGET` 来自二进制内嵌的 Rust target，例如 `x86_64-pc-windows-msvc`、`x86_64-unknown-linux-gnu`、`aarch64-apple-darwin`。具体提供的平台以该版本附件为准；GNU Linux 包的系统运行库要求应在发布说明中注明。

先核对压缩包 SHA-256，再解压。Linux 可使用 `sha256sum -c FILE.zip.sha256`，macOS 可使用 `shasum -a 256 -c FILE.zip.sha256`。Windows 使用 `Get-FileHash FILE.zip -Algorithm SHA256`，与校验文件首列比较。将 `okf` 或 `okf.exe` 放入 PATH 中的目录；Unix 解压后必要时执行 `chmod +x okf`。

执行 `okf --version` 与 `okf build-info --format json` 核对版本和平台。`manifest.json` 记录版本、构建提交、源码是否有未提交修改、tag 和每个打包文件的哈希。构建时没有 Git 时来源字段为 null；正式 tag 包要求来源已知、源码干净且提交匹配。

## 升级与回退

先阅读随包的 [变更记录](../CHANGELOG.md)，保留旧二进制，再校验和替换程序。知识包保持只读，程序替换不会自动刷新 Git 绑定或同步正文索引。涉及配置或数据库格式的版本应先备份运行目录，再按该版本迁移说明操作；当前不承诺任意版本间数据库向后兼容。

内置消费 Skill 随二进制发行。升级 Skill 时，先安装到新目录，对比并合并已有定制；`skill install` 保留防覆盖行为。

## 本地发行验收

```sh
cargo build --release --locked -p okf-cli
python -m pip install -r scripts/requirements-test.txt
python scripts/check_delivery.py target/release/okf
```

Windows 使用 `target/release/okf.exe`。验收检查 Schema、打包拒绝条件、本地 HTTPS Git、检索及跨包基线，并在解压包中核对哈希、构建信息和内置 Skill 后运行独立验收。产物位于 `target/dist`，本地开发包允许记录 `source_dirty: true`。

准备正式版本时，先统一 workspace 版本、Cargo.lock 与变更记录，提交到干净工作区，再创建对应 `vVERSION` tag。带 tag 验收：

```sh
cargo build --release --locked -p okf-cli
python scripts/check_delivery.py target/release/okf --tag vVERSION
```

此处 `vVERSION` 为占位符，必须替换成实际版本。提交并创建 tag 后，重新执行上述 Release 构建，再进行带 tag 验收，确保二进制记录的是干净工作区的当前提交。脚本要求 tag、当前 HEAD、二进制构建提交一致，并拒绝版本、平台或内置 Skill 不匹配的包。

打包和解压校验使用显式检查，Python `-O` 模式也会执行。`check_delivery.py` 包含两种模式下的 8 类拒绝场景，覆盖版本、平台、tag、内置 Skill、归档哈希、路径、文件清单和内容哈希。

## 发布流程

`Package binaries` 工作流在 tag 推送和手动运行时生成三平台产物；tag 构建自动执行上述严格校验。手动运行时选择版本 tag 并启用 `create_draft`，三平台验收通过后才创建带附件的 GitHub Release 草稿。分支运行不能创建草稿。工作流不会覆盖已存在的同名 Release，也不自动发布草稿。

正式发布前检查草稿附件、支持平台、系统依赖和变更说明，再单独发布。仅打包任务具有只读仓库权限；创建草稿的任务单独获得 contents 写权限。
