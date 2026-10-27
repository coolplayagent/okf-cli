# 安装、升级与发行

CLI 源码构建方法见 [README](../README.md#安装与构建)。Python 3.11 及以上仅用于开发和打包，已构建的 CLI 无需 Python 或 Rust；读取 Git 知识包仍需要 Git。

## 下载与安装

当前正式版本为 `v0.1.0`；`v0.1.0-alpha.1` 保留为历史预览版本。正式版沿用预览版的功能，没有数据库格式变更。后续版本仍需按各自迁移说明升级，不承诺任意版本间数据库兼容；升级前请备份运行目录。

发行目标为 Windows x86_64 MSVC、Linux x86_64 GNU 和 macOS ARM64。Linux 要求 glibc 2.35 或更新版本；Windows 需要 VC/UCRT 运行库；macOS deployment target 为 14.0。实际验收环境为 Windows Server 2022、Ubuntu 22.04 和 macOS 15，最低系统版本尚未实机验证。

支持本地文件系统和公开 HTTPS Git 知识包；私有仓库认证、SSH、submodule 和 LFS 下载不在本版范围内。确定性检索验收不代表真实 Agent 答案质量已经测量。

已发布版本从 [GitHub Releases](https://github.com/coolplayagent/okf-cli/releases) 下载。工作流产物或 Release 草稿不等同于正式发布。

选择与系统、CPU 和运行环境相符的 `okf-cli-VERSION-TARGET.zip` 及同名 `.zip.sha256`。`TARGET` 来自二进制内嵌的 Rust target，例如 `x86_64-pc-windows-msvc`、`x86_64-unknown-linux-gnu`、`aarch64-apple-darwin`。具体提供的平台以该版本附件为准；GNU Linux 包的系统运行库要求应在发布说明中注明。

先核对压缩包 SHA-256，再解压。Linux 可使用 `sha256sum -c FILE.zip.sha256`，macOS 可使用 `shasum -a 256 -c FILE.zip.sha256`。Windows 使用 `Get-FileHash FILE.zip -Algorithm SHA256`，与校验文件首列比较。将 `okf` 或 `okf.exe` 放入 PATH 中的目录；Unix 解压后必要时执行 `chmod +x okf`。

执行 `okf --version` 与 `okf build-info --format json` 核对版本和平台。`manifest.json` 记录版本、构建提交、源码是否有未提交修改、tag 和每个打包文件的哈希。构建时没有 Git 时来源字段为 null；正式 tag 包要求来源已知、源码干净且提交匹配。

## 升级与回退

先阅读随包的 [变更记录](../CHANGELOG.md)，保留旧二进制，再校验和替换程序。知识包保持只读，程序替换不会自动刷新 Git 绑定或同步正文索引。涉及配置或数据库格式的版本应先备份运行目录，再按该版本迁移说明操作；当前不承诺任意版本间数据库向后兼容。

内置消费 Skill 随二进制发行。升级 Skill 时，先安装到新目录，对比并合并已有定制；`skill install` 保留防覆盖行为。

### Skill 更名与升级

2026-10-27 重建的 `v0.1.0` 将消费 Skill 从 `okf-consume` 更名为 `okf-cli`：源码与新构建包中的路径为 `skills/okf-cli/SKILL.md`，文件 frontmatter 为 `name: okf-cli`，并新增独立 Skill ZIP。此次重建更新了 `v0.1.0` 标签；此前下载的同版本包可能仍使用旧名称，需通过 `build-info` 的来源提交和发行附件哈希区分。CLI 可执行文件仍为 `okf`，知识包绑定和数据库格式不受此改名影响。

从当前源码重新构建并使用新二进制执行：

```sh
okf skill install .agents/skills/okf-cli
```

确认安装文件中的 `name: okf-cli`，将旧目录中的定制合并到新文件，再更新 Agent 配置或提示中的 Skill 名称与路径。确认新 Skill 可用后，将旧目录移出 Agent 的技能发现目录，避免同时加载两份。安装命令不会自动迁移或删除旧目录；目标已有 `SKILL.md` 时会拒绝覆盖，可先安装到临时目录后比较合并。使用旧二进制安装到新目录仍会得到旧名称。

## 本地发行验收

```sh
cargo build --release --locked -p okf-cli
python -m pip install -r scripts/requirements-test.txt
python scripts/check_delivery.py target/release/okf
```

Windows 使用 `target/release/okf.exe`。验收范围见 [开发检查](../README.md#开发)，包括索引完整性与修复、有限预算评测，以及解压包的哈希、构建信息、内置 Skill 和独立运行检查。产物位于 `target/dist`，本地开发包允许记录 `source_dirty: true`。

准备正式版本时，先统一 workspace 版本、Cargo.lock 与变更记录，提交到干净工作区，再创建对应 `vVERSION` tag。带 tag 验收：

```sh
cargo build --release --locked -p okf-cli
python scripts/check_delivery.py target/release/okf --tag vVERSION
```

此处 `vVERSION` 为占位符，必须替换成实际版本。提交并创建 tag 后，重新执行上述 Release 构建，再进行带 tag 验收，确保二进制记录的是干净工作区的当前提交。脚本要求 tag、当前 HEAD、二进制构建提交一致，并拒绝版本、平台或内置 Skill 不匹配的包。

打包和解压校验使用显式检查，Python `-O` 模式也会执行。`check_delivery.py` 包含两种模式下的 8 类拒绝场景，覆盖版本、平台、tag、内置 Skill、归档哈希、路径、文件清单和内容哈希。

## 发布流程

产物组织参考 [relay-knowledge](https://github.com/coolplayagent/relay-knowledge/blob/main/.github/workflows/release.yml) 和 [into-markdown](https://github.com/coolplayagent/into-markdown/blob/main/.github/workflows/platform-modular-release.yml) 的平台包与独立 Skill 汇总方式。本项目继续使用上述三个发行目标和系统依赖基线。

每次构建在三平台验收成功后，额外生成 `release-downloads` 工作流产物，包含三个原有 CLI ZIP、`okf-cli-skill-VERSION.zip`、四份对应的 `.zip.sha256`、`release-set.json` 及 `SHA256SUMS`。汇总拒绝缺失或多余的平台包、校验和错误、文件清单不一致、不同提交、脏源码和 tag 不匹配；分支构建也要求所有包来自同一干净提交。`release-set.json` 记录四个下载包的哈希及来源，`SHA256SUMS` 覆盖其他全部附件。下载后可运行 `sha256sum -c SHA256SUMS`（macOS 使用 `shasum -a 256 -c SHA256SUMS`）。这些校验和用于完整性检查，不是发布者签名。

Agent 用户可将独立 Skill ZIP 解压到技能目录，得到 `okf-cli/SKILL.md` 和 `assets/` 下的 Windows x64、Linux x64、macOS ARM64 二进制，无需先安装 Rust、Python 或将 `okf` 加入 PATH。Skill 按宿主平台选择随包二进制，并保留用户项目工作目录。Unix 解压后若执行位丢失，执行 `chmod +x`；Git 和系统运行库要求与 CLI 包相同。`okf skill install` 仍只安装指令文件，完整离线包应直接解压。

本地汇总已通过平台验收的三个包（输入目录只放 ZIP 及其 `.sha256`，输出目录必须为空）：

```sh
python scripts/assemble_release.py INPUT_DIR OUTPUT_DIR --commit SOURCE_COMMIT
```

正式 tag 包追加 `--tag vVERSION`。该步骤验证归档内容与来源，不在汇总主机执行其他架构的程序。汇总后工作流还会在三个原生平台分别运行 `python scripts/verify_skill_package.py SKILL_ZIP`，核对 Skill 内文件哈希、二进制构建信息与内置指令，并在不含编译工具的 PATH 下执行冒烟验收；三平台均通过后才允许创建草稿。

`Package binaries` 工作流在 tag 推送和手动运行时生成三平台产物；tag 构建自动执行上述严格校验。手动运行时选择版本 tag 并启用 `create_draft`，三平台验收通过后才创建带附件的 GitHub Release 草稿。分支运行不能创建草稿。工作流不会覆盖已存在的同名 Release，也不自动发布草稿。

草稿附件来自汇总验证后的 `release-downloads`。正式发布前检查草稿附件、支持平台、系统依赖和变更说明，再单独发布。打包与汇总任务均使用只读仓库权限；创建草稿的任务单独获得 contents 写权限。

## 三平台打包实测

2026-10-26 对提交 `ab4f735` 手动执行 [Package binaries](https://github.com/coolplayagent/okf-cli/actions/runs/40263932075)，三平台构建、交付验收与产物上传全部通过。下载后再次核对了三个压缩包及包内文件哈希，manifest 均记录同一提交和干净源码。这是历史工作流产物验收，未创建 tag、Release 草稿或正式版本；不包含该提交之后的功能与发行流程变更。最新验证范围见 [实现状态](implementation-status.md#最近验证)。

| 产物目标 | 本次二进制的运行库要求 |
| --- | --- |
| `x86_64-unknown-linux-gnu` | ELF 版本依赖要求最高为 `GLIBC_2.39`，还依赖 `libgcc_s.so.1`（含 `GCC_4.2.0` 符号版本）、`libm.so.6` 和系统动态加载器 |
| `x86_64-pc-windows-msvc` | 依赖 `VCRUNTIME140.dll`、UCRT 与 Windows 系统 DLL；无需 Rust 或 Python |
| `aarch64-apple-darwin` | Mach-O deployment target 为 macOS 11.0，依赖系统 Foundation/CoreFoundation、libSystem、libobjc 和 libiconv |

这些信息来自历史下载包的依赖记录，不代表下述新构建基线。CI 运行成功只验证对应 runner 环境，deployment target 不等于已在最低系统版本实测。

`python scripts/check_upgrade.py BEFORE AFTER` 在隔离目录中安装旧二进制、绑定并同步知识包、替换为新二进制后离线读取同一 generation，再回退并复核绑定与原文。CLI 子进程 PATH 不包含编译工具；Python 仅驱动验收。该流程验证指定二进制对和数据库格式，不承诺任意版本间可回退。

## 固定发行基线

打包工作流固定 Rust 1.99.1，并使用以下平台标签。标签来自 [GitHub runner 镜像清单](https://github.com/actions/runner-images#available-images)；平台补丁镜像仍会更新，因此这不构成逐字节可重现构建。

| Rust target | 构建及验收环境 | 自动依赖检查 |
| --- | --- | --- |
| `x86_64-unknown-linux-gnu` | Ubuntu 22.04 | `readelf` 检查 glibc 符号版本不高于 2.35，并记录动态库 |
| `x86_64-pc-windows-msvc` | Windows Server 2022 | 读取 PE 导入表，记录 VC/UCRT 与系统 DLL |
| `aarch64-apple-darwin` | macOS 15 ARM64 | 设置 deployment target 14.0，使用 `otool` 检查并记录系统库 |

`python scripts/check_runtime.py BINARY OUTPUT.json` 输出实际二进制的构建信息、检查宿主和依赖。Windows GNU 开发构建也可检查，但标记为非发行目标。Windows 桌面最低版本、缺少 VC 运行库的机器以及 macOS 14 的实际运行尚未验证，报告保留 `minimum_os_verified: false`。

工作流另行构建固定基线 `cae1cff309941735326fcdb79b953c640150ae23`，在各平台执行升级、离线读取和回退。运行库报告随验收产物上传。2026-10-26 对合并提交 `c064e39` 执行的 [打包运行 40292399047](https://github.com/coolplayagent/okf-cli/actions/runs/40292399047) 已在上述三个环境完成全部检查；三个下载包及清单哈希再次通过，Windows 包另经独立运行和 65 份语料、184 次 CLI 调用复核。

该次 Linux 包最高 glibc 符号依赖为 2.34，macOS deployment target 为 14.0，Windows 包记录 VC/UCRT 与系统 DLL。实际运行证据对应表中的 runner 系统，仍不证明最低操作系统可用。该产物不包含后续生产候选检索改动；版本 tag 和草稿仍需依照发布流程单独验收。
