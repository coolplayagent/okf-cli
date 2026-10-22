# okf-cli

面向 Agent 的 [Open Knowledge Format（OKF）](https://github.com/GoogleCloudPlatform/open-knowledge-format/blob/main/SPEC.md) 知识读取工具。注册知识包，通过概览、目录、检索和原文读取逐步获取任务所需的知识。

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

使用 `--format json` 获取机器可读结果。响应包含知识包、覆盖范围、诊断和后续操作；分页时将 `page.next_cursor` 传入同一命令的 `--cursor` 参数继续读取。参数说明可通过 `okf --help` 或 `okf <命令> --help` 查看。

## 配置

知识包绑定保存在项目 `.okf-cli/config.toml` 或用户数据目录中。使用 `--scope project` 或 `--scope user` 选择作用域，通过 `OKF_HOME` 或 `--home` 指定绝对运行目录。

项目内的相对知识包路径按项目根目录解析。解除绑定不会删除知识包文件：

```sh
okf bundle remove handbook --scope project
```

## Agent Skill

将消费 Skill 安装到 Agent 使用的技能目录：

```sh
okf skill install .agents/skills/okf-consume
```

Skill 引导 Agent 发现知识包、检索候选概念、按需读取原文并保留知识包与生命周期信息。已有的 `SKILL.md` 不会被覆盖。

## 开发

```sh
cargo fmt --all -- --check
cargo test --workspace --all-targets --locked
cargo test --workspace --all-targets --release --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

## 文档

- [产品与架构](docs/architecture.md)：模块边界、数据流和存储职责。
- [CLI 与 Agent 参考](docs/cli-contract.md)：命令参数、响应结构和消费流程。
- [实现状态](docs/implementation-status.md)：功能范围、行为边界和验证方法。
- [实施计划](docs/roadmap.md)：开发阶段与验收目标。

## 许可证

[MIT](LICENSE)
