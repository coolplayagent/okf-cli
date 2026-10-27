# okf-cli

当前正式版本为 `0.1.0`，支持本地文件系统和公开 HTTPS Git 知识包的只读消费。支持平台、安装方法、升级兼容范围和已知限制见 [发行说明](docs/releases.md)。

面向 Agent 的 [Open Knowledge Format（OKF）](https://github.com/GoogleCloudPlatform/open-knowledge-format/blob/main/SPEC.md) 知识消费 CLI。注册知识包，通过概览、目录、检索和原文读取逐步获取任务所需的知识。

知识包内容保持只读，读取结果提供知识包、文件定位和后续导航，便于 Agent 继续探索和引用。

## 安装与构建

需要 Rust stable 和 C 编译环境。在仓库根目录执行：

```sh
cargo install --locked --path apps/okf-cli
okf --help
```

确保 Cargo 的 `bin` 目录已加入 PATH。也可以直接构建可执行文件：

```sh
cargo build --release --locked -p okf-cli
```

产物位于 `target/release/okf`，Windows 下为 `target/release/okf.exe`。

发行包的下载、校验、安装和升级步骤见 [发行说明](docs/releases.md)。`okf build-info --format json` 可离线查看程序版本、Rust 目标平台和构建来源。

### Windows 工具链

Windows 可选择 MSVC 或 GNU 工具链。使用 MSVC 时，需要安装 C++ Build Tools 和 Windows SDK。

使用 GNU 时，将便携式 MinGW GCC（例如 [w64devkit](https://github.com/skeeto/w64devkit)）的 `bin` 目录加入 PATH，然后为当前仓库选择 GNU Rust：

```powershell
rustup toolchain install stable-x86_64-pc-windows-gnu --profile minimal --component rustfmt,clippy
rustup override set stable-x86_64-pc-windows-gnu
cargo install --locked --path apps/okf-cli
```

## 快速使用

在仓库根目录绑定示例知识包：

```sh
okf bundle add handbook --kind fs --path ./examples/knowledge --scope project
```

查看知识包概览，展开目录，再读取具体概念：

```sh
okf context --bundle handbook --format json
okf browse @handbook/services --format json
okf read @handbook/services/auth --format json
```

检索概念、检查知识包或查看知识包状态：

```sh
okf search "认证" --bundle handbook --format json
okf validate handbook --format json
okf status handbook --format json
```

查询可用 `--as-of 2030-01-01T00:00:00Z` 固定有效期评估时间；`okf validate handbook --max-warnings 0` 可启用质量门槛。参数和不完整结果的处理见 [CLI 参考](docs/cli-contract.md)。

### Git 知识包

绑定 Git 仓库的指定提交视图，分支更新后显式刷新：

```sh
okf bundle add upstream --kind git --path ../knowledge-repo --ref main --bundle-path .okf
okf bundle refresh upstream
```

Git 适配器需要 Git 可执行文件，可用 `OKF_GIT` 指定路径。也可用 `--url https://github.com/OWNER/REPO.git` 替代 `--path` 绑定公开 HTTPS 仓库。读取使用固定 commit 的 tree/blob，不检出工作区。

需要代理时，通过 `HTTPS_PROXY` / `HTTP_PROXY` 传给 Git；不会自动读取操作系统浏览器代理。私有仓库凭据配置暂未提供。

### 正文索引与离线读取

显式同步正文索引后，可检索正文、选择上下文片段或离线读取：

```sh
okf sync handbook --max-scan 10000
okf search "认证" --bundle handbook --full-text --format json
okf context --query "认证" --bundle handbook --content --format json
okf read @handbook/services/auth --indexed --format json
okf index status handbook
okf index verify handbook --max-scan 10000 --format json
okf search "认证" --bundle handbook --full-text --type Service --tag security
okf search "认证" --bundle handbook --full-text --dir services --max-scan 10
okf search "认证" --bundle handbook --full-text --candidate-budget 10000 --max-scan 10
okf search "认证" --bundle handbook --full-text --trust human-reviewed --expiry current
okf neighbors @handbook/services/auth --direction incoming --format json
```

全文检索与上下文选择可重复指定 `--bundle` 查询多个知识包，共用请求预算；各结果保留自己的 generation、内容哈希和原文字节范围。离线索引代表上次成功同步的内容。扫描不完整时不会替换旧索引。

`search` 和带查询的 `context` 支持 `--dir PATH`，限定包内目录及其子目录；索引模式先限定目录再分配扫描预算。结果总数与完整性只描述所选范围。

索引查询可用 `--candidate-budget N` 启用持久候选检索，先筛选候选再检查正文；默认仍按路径扫描。旧索引先重新 `sync`，候选行预算与正文扫描预算分别计量，详见 [候选检索](docs/cli-contract.md#候选检索)。

使用 `okf cache reset` 将历史卡片缓存归档后重建；`okf index clear NAME` 清理指定知识包的正文索引。两者均保留知识包与绑定。

`index verify` 只读检查索引路径、哈希、文档版本、快照清单及已有的候选派生数据，不访问源文件。检查不完整且尚未发现问题时，`data.valid` 为 null；已发现损坏时为 false。使用 `sync NAME` 从源文件重新发布完整索引以修复损坏，详细语义见 [索引完整性检查](docs/cli-contract.md#索引完整性检查)。

使用 `--format json` 获取机器可读结果。目录和原文分页时，将 `page.next_cursor` 传入同一命令的 `--cursor` 参数继续读取。可通过 `okf <命令> --help` 查看选项，通过 [CLI 参考](docs/cli-contract.md) 了解预算、诊断和覆盖范围。

## 配置

知识包绑定保存在项目 `.okf-cli/config.toml` 或用户数据目录中。使用 `--scope project` 或 `--scope user` 选择作用域，通过 `OKF_HOME` 或 `--home` 指定绝对运行目录。

项目内的相对知识包路径按项目根目录解析。解除绑定不会删除知识包文件：

```sh
okf bundle remove handbook --scope project
```

## Agent Skill

当前源码中的消费 Skill 名为 `okf-cli`，目录为 [`skills/okf-cli`](skills/okf-cli/SKILL.md)。使用从当前源码构建的二进制，将其安装到 Agent 使用的技能目录：

```sh
okf skill install .agents/skills/okf-cli
```

Skill 引导 Agent 发现知识包、检索候选概念、按需读取原文并保留知识包与生命周期信息。已有的 `SKILL.md` 不会被覆盖。

此改名尚未发布，已发布的 `v0.1.0` 二进制仍内置 `okf-consume`；仅改变安装目录不会改变内置 Skill 的名称。已有安装的迁移步骤见 [Skill 更名与升级](docs/releases.md#skill-更名与升级)。

## 开发

```sh
cargo fmt --all -- --check
cargo test --workspace --all-targets --locked
cargo test --workspace --all-targets --release --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Git 测试需要 Git。交付验证的 Python 依赖仅用于开发与 CI，CLI 运行时不需要 Python：

```sh
python -m pip install -r scripts/requirements-test.txt
cargo build --release --locked -p okf-cli
python scripts/check_delivery.py target/release/okf
python scripts/evaluate.py target/release/okf target/verification/retrieval-1000.json --scale 1000
```

`check_delivery.py` 校验响应 Schema、目录范围、索引完整性与同步修复、本地 HTTPS Git 流程与故障、检索预算、12 个消费任务回放、固定语料哈希及发行拒绝条件，并在解压后二进制上执行独立运行与上游语料验收。CI 与打包工作流共用该入口；验收不访问公网。

固定版本的上游语料位于 `fixtures/upstream`。需要单独测量性能基线时执行：

```sh
cargo run --release --locked -p okf-adapters --example baseline -- fixtures/upstream
```

Windows 将二进制路径改为 `target/release/okf.exe`。打包结果包含校验和，位于 `target/dist`；tag 验收和草稿发布流程见 [发行说明](docs/releases.md#本地发行验收)。

上游语料验收在临时目录中检查实时与离线读取、Unicode 分页、索引版本失效、索引正文修复和观测缓存归档恢复，原始语料保持不变。最近验证结果见 [实现状态](docs/implementation-status.md#最近验证)。

## 文档

- [产品与架构](docs/architecture.md)：模块边界、数据流和存储职责。
- [CLI 与 Agent 参考](docs/cli-contract.md)：命令参数、响应结构和消费流程。
- [OKF 检查矩阵](docs/okf-validation.md)：规范依据、诊断级别和字段定位。
- [上游兼容检查](docs/upstream-compatibility.md)：固定语料、在线差异报告和基线更新流程。
- [实现状态](docs/implementation-status.md)：功能范围、行为边界和验证方法。
- [性能基线](docs/performance.md)：固定上游语料、测量方法与结果边界。
- [检索任务评测](docs/retrieval-evaluation.md)：预期证据、章节选择、调用量与内存测量。
- [Agent 消费评测](docs/agent-evaluation.md)：隔离任务准备、对照策略与答案质量评分协议。
- [实施计划](docs/roadmap.md)：剩余工作与验收目标。
- [发行说明](docs/releases.md)：安装、升级、版本校验和草稿发布流程。
- [变更记录](CHANGELOG.md)：未发布变更与已合并基础能力。

## 许可证

[MIT](LICENSE)
