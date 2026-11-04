# 验证与评测

本文供测试与维护者执行回归、复现兼容性和性能评测，并查阅历史证据。功能边界见 [实现状态](architecture.md#实现状态)，参数和错误语义见 [CLI 参考](cli-contract.md)。历史结果只适用于当时的源码、环境与语料；配置了工作流不等于当前改动已通过远端验收。

- [功能与文档逐项对照](#功能与文档逐项对照)
- [回归入口](#验证入口)与[验收边界](#自动化与验收边界)
- [上游兼容检查](#上游兼容检查)
- [性能基线](#性能基线与兼容语料)
- [检索任务评测](#检索任务评测)
- [Agent 答案质量协议](#agent-cli-消费评测协议)
- [历史验证记录](#最近验证)

## 验证入口

在仓库根目录执行基础检查：

```sh
cargo fmt --all -- --check
cargo test --workspace --all-targets --locked
cargo test --workspace --all-targets --release --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

交付检查使用实际构建的二进制；Windows 将路径改为 `target/release/okf.exe`：

```sh
python -m pip install -r scripts/requirements-test.txt
cargo build --release --locked -p okf-cli
python scripts/check_delivery.py target/release/okf
```

Git 测试需要 Git；HTTPS 脚本使用本地 TLS 服务。交付验收不访问公网，Python 仅为开发依赖。打包与解压验证会生成 `target/dist` 和 `target/verification` 产物。发行 tag 检查见 [本地发行验收](releases.md#本地发行验收)。

| 维度 | 验证入口 | 可以证明的范围 |
| --- | --- | --- |
| 模型与应用行为 | Rust workspace 测试 | 解析、诊断、预算、排序、分页及故障回归 |
| 响应契约 | `scripts/check_schema.py`（包含 `check_search_schema.py`） | 实际响应及元数据搜索卡片的必填字段、类型、空值和扩展兼容；包含隔离的合成反例，不代表每个业务字段均有完整 Schema |
| 索引与目录范围 | `scripts/check_index.py`、`scripts/check_scope.py` | 完整性、修复、离线读取与目录范围隔离 |
| 候选检索 | `scripts/evaluate_index_candidates.py` | 实际 CLI 与路径扫描结果对照、预算、损坏、兼容和重同步 |
| Git 传输与缓存 | `scripts/check_git_https.py`、`scripts/check_git_integrity.py` | 本地真实 HTTPS 传输、凭据隔离、失败恢复、子模块和 LFS 完整性 |
| 检索与消费 | `scripts/evaluate.py`、`evaluate_multi_bundle.py`、`evaluate_budgets.py`、`evaluate_consumption.py` | 固定任务证据、覆盖和调用轨迹；不测量模型答案质量 |
| Agent 评测准备 | `scripts/check_agent_preparation.py` | 隔离项目、盲测任务和拒绝覆盖；不调用模型 |
| 上游与发行 | `scripts/check_upstream.py`、`check_package.py`、`verify_package.py` | 固定语料、打包拒绝条件、解压包独立运行及语料验收 |

表中 Python 入口由 [交付脚本](../scripts/check_delivery.py) 组织。性能与大规模检索是另行执行的测量，见 [性能基线](#性能基线与兼容语料) 和 [检索评测](#检索任务评测)；在线上游差异检查见 [兼容检查](#上游兼容检查)，不属于默认离线验收。

## 功能与文档逐项对照

2026-11-03 对当前未提交源码逐项核对。清单以 `apps/okf-cli/src/main.rs` 的 21 个业务叶子命令为入口，再覆盖组合模式和横切能力。下表表示实现与说明已对照，不表示本轮重新执行全部测试，也不证明已发布二进制具备当前功能。功能变化时同步维护对应契约章节、回归入口和变更记录，执行后再补充验证证据。

| 命令 | 核对的功能与边界 | 文档定位 | 实现及回归入口 |
| --- | --- | --- | --- |
| `bundle add` | fs/Git、本地/远端、默认 ref 与包根、凭据、配额、重复别名及作用域 | [绑定与管理](cli-contract.md#绑定与管理操作边界)、[Git](cli-contract.md#git-远端与缓存) | `main.rs`、`config.rs`、`git.rs`；CLI 注册、重复别名及 HTTPS 回归 |
| `bundle list` | 配置列表、排序、作用域；不逐包探测 | [绑定与管理](cli-contract.md#绑定与管理操作边界) | `main.rs`、`ConfigRegistry::list`；CLI 作用域回归 |
| `bundle show` | 单绑定选择、歧义、固定提交与凭据引用 | [命令](cli-contract.md#命令)、[绑定与管理](cli-contract.md#绑定与管理操作边界) | `main.rs`、`App::binding`；CLI 注册回归 |
| `bundle remove` | 只解绑、其他作用域及源文件保留 | [绑定与管理](cli-contract.md#绑定与管理操作边界) | `ConfigRegistry::remove`；CLI 删除与作用域回归 |
| `bundle refresh` | 本地 ref 重解析、远端获取、原子替换、失败保留旧提交 | [绑定与管理](cli-contract.md#绑定与管理操作边界)、[Git](cli-contract.md#git-远端与缓存) | `main.rs`、`ConfigRegistry::replace`；CLI Git 刷新、HTTPS 回归 |
| `bundle cache` | 占用查询、压缩、GC 优先级、托管路径及可见绑定保护范围 | [Git](cli-contract.md#git-远端与缓存) | `Git::cache/gc`；Git 压缩、嵌套子模块及 GC 回归 |
| `context` | 概览的包数预算、元数据查询、正文上下文、多包与参数约束 | [命令](cli-contract.md#命令)、[参数组合](cli-contract.md#参数组合)、[搜索](cli-contract.md#搜索与生命周期) | `service.rs`、`index.rs`；CLI 多包、章节及消费任务回放 |
| `browse` | 一层目录、作者索引、排序、扫描预算与 cursor | [引用与原文](cli-contract.md#引用与原文)、[预算与分页](cli-contract.md#预算与分页) | `App::browse`；CLI 目录分页、修订及扫描不足回归 |
| `read` | 实时、raw、离线、generation、UTF-8 原文分页与互斥 | [引用与原文](cli-contract.md#引用与原文)、[参数组合](cli-contract.md#参数组合) | `App::read/indexed_read`；CLI 原文、Unicode、离线及版本回归 |
| `search` | 字段值匹配、空查询、全局排序、正文评分、筛选、多包、片段 | [搜索](cli-contract.md#搜索与生命周期)、[候选检索](cli-contract.md#候选检索) | `service.rs`、`index.rs`、`candidates.rs`；CLI 搜索及生产候选对照 |
| `status` | 历史卡片观测统计；不扫描、不判断正文索引新鲜度 | [绑定与管理](cli-contract.md#绑定与管理操作边界) | `main.rs`、`SqliteCache::status`；缓存回归及本轮管理冒烟 |
| `neighbors` | 入链/出链、直接证据、来源路径、缺失目标、扫描与选择覆盖 | [关系导航](cli-contract.md#关系导航与扩展检查) | `neighbors.rs`；CLI 循环、断链、限制回归 |
| `validate` | 结构、保留文件、元数据、正文、计算资源、附件、锚点和质量门槛 | [检查矩阵](cli-contract.md#okf-检查矩阵)、[退出码](cli-contract.md#退出码) | `service.rs`、`resources.rs`、`quality.rs`；核心固定样例与 CLI 校验回归 |
| `doctor` | 运行状态信息及缓存打开失败边界；不验证全部内容或网络 | [绑定与管理](cli-contract.md#绑定与管理操作边界) | `main.rs`；本轮管理冒烟、Schema 验收入口 |
| `build-info` | 构建来源、未知值、无需配置与数据库 | [绑定与管理](cli-contract.md#绑定与管理操作边界) | `main.rs`、构建脚本；CLI 无运行状态回归 |
| `sync` | 保留文件入库、附件不入库、事务发布、失败保留、同版本修复 | [搜索](cli-contract.md#搜索与生命周期)、[完整性检查](cli-contract.md#索引完整性检查) | `App::sync`、`SqliteIndex`；CLI 离线/删除同步及索引事务回归 |
| `index status` | 快照统计、Git 修订比较；不代替完整性检查 | [搜索](cli-contract.md#搜索与生命周期) | `App::index_status`；索引选择性读取、本轮管理冒烟 |
| `index clear` | 仅删除所选绑定的正文与派生数据 | [绑定与管理](cli-contract.md#绑定与管理操作边界) | `main.rs`、`SqliteIndex::clear`；本轮清理后缺失索引及源文件读取探针 |
| `index verify` | 只读、哈希/版本/清单、候选重算、预算、三值有效性 | [完整性检查](cli-contract.md#索引完整性检查) | `App::index_verify`；索引 Rust 回归、`check_index.py` |
| `cache reset` | 观测缓存归档重建、sidecar、活动读取者拒绝 | [绑定与管理](cli-contract.md#绑定与管理操作边界)、[故障与覆盖](cli-contract.md#故障与覆盖) | `SqliteCache::reset`；缓存恢复与活动消费者回归 |
| `skill install` | 内嵌入口与按需参考、自动建目录、两份文件防覆盖、无运行状态依赖 | [命令](cli-contract.md#命令)、[发行升级](releases.md#skill-更名与升级) | `main.rs`；CLI 安装防覆盖、打包验收 |

代码简称中，`main.rs` 为 CLI 入口，`service.rs`、`index.rs`、`neighbors.rs`、`resources.rs`、`quality.rs` 位于 `crates/okf-application/src`；配置、Git 与 SQLite 适配器位于 `crates/okf-adapters/src`。CLI 回归位于 `apps/okf-cli/tests/cli.rs`，适配器回归位于 `crates/okf-adapters/tests`。

| 横切功能或组合模式 | 核对结果与文档定位 | 证据入口 |
| --- | --- | --- |
| 全局参数、帮助与版本 | [全局选项](cli-contract.md#全局选项) 覆盖作用域、运行目录优先级、文本帮助及输出格式；[参数组合](cli-contract.md#参数组合) 区分适用命令 | 21 个实际帮助页、40 个长选项；CLI 参数拒绝回归 |
| 预算、分页、取消、输出上限 | [预算与分页](cli-contract.md#预算与分页)、[截止时间](cli-contract.md#截止时间与取消)、[JSON 响应](cli-contract.md#json-响应) | CLI 有界 JSON、Unicode cursor、应用请求控制回归 |
| 文件路径与一致性 | [路径与一致性](architecture.md#路径与一致性) 说明观测视图、目录链接/Junction、大小写冲突和原文修订 | filesystem 适配器与 CLI 路径回归 |
| 元数据筛选与有效期 | [搜索](cli-contract.md#搜索与生命周期)、[派生规则](cli-contract.md#派生与覆盖) 区分文本字段、筛选、请求时间与源文件声明 | 核心 metadata 固定样例、CLI 生命周期回归 |
| 持久候选与旧库 | [候选检索](cli-contract.md#候选检索) 覆盖版本 2、行预算、容量回退、旧库重同步 | candidates 适配器回归、生产候选 32 组对照入口 |
| Git 凭据、SSH、LFS、子模块 | [Git](cli-contract.md#git-远端与缓存) 区分获取、离线读取、认证继承与配额范围 | Git Rust 回归、本地 HTTPS、LFS 完整性脚本；真实 SSH 服务仍未验收 |
| OKF 元数据、链接、计算、附件、锚点 | [检查矩阵](cli-contract.md#okf-检查矩阵) 覆盖规则、诊断、位置、截断与可选检查 | 核心 metadata/computation/markdown 样例、sections 模块及 CLI 校验回归 |
| 故障隔离与质量门槛 | [故障与覆盖](cli-contract.md#故障与覆盖)、[退出码](cli-contract.md#退出码) 区分部分结果、缓存故障、未知统计与门槛 | CLI 多包故障、缓存、warning gate 回归 |
| 发行、升级与评测工具 | [安装与发行](releases.md) 和本文各评测章节说明操作流程及历史证据，不混同源码能力与发行能力 | 打包工作流、交付入口、升级脚本；本轮未重新执行跨平台发行 |

## 自动化与验收边界

[CI](../.github/workflows/ci.yml) 和 [打包工作流](../.github/workflows/package.yml) 定义跨平台检查及产物上传。当前工作区是否通过，以对应提交的实际运行结果为准。升级回退由 `scripts/check_upgrade.py` 对指定的前后两个二进制单独验证；不能用一次旧版本对照证明任意版本兼容。

真实 SSH 服务、最低系统版本、真实 Agent 答案评分均需要独立环境或执行协议；现有本地 HTTPS、runner 测试和任务准备不能替代这些验收。

新增验证记录应写明日期、源码提交或未提交状态、平台、构建类型、命令、结果与未覆盖范围。本地产物路径用于定位执行记录，通常不纳入版本控制，不保证其他检出中存在。

## 上游兼容检查

### 官方完整语料验收

`scripts/check_official_corpus.py` 使用现有 CLI 二进制和官方仓库 checkout，隔离验证校验、浏览、原文字节与哈希、分页、搜索、同步及离线索引读取，不调用模型。需要 Python 3.11+ 和 Git；Git 不在 PATH 时可设置 `OKF_GIT`。目前支持 `ga4`、`stackoverflow`、`crypto_bitcoin` 和 `acme_retail`，未知目录会明确失败，可用 `--bundles` 选择支持的目录。空选择、拼错名称和没有 Markdown 文档均不会算作通过。

先将官方仓库 checkout 到要验证的固定提交，再运行（输出目录必须尚不存在）：

```sh
git -C /path/to/open-knowledge-format checkout --detach <commit-sha>
python scripts/check_official_corpus.py target/release/okf /path/to/open-knowledge-format target/verification/official-fs --as-of 2026-10-28T00:00:00Z
python scripts/check_official_corpus.py target/release/okf /path/to/open-knowledge-format target/verification/official-git --git-url https://github.com/GoogleCloudPlatform/open-knowledge-format.git --ref <commit-sha> --bundles ga4
```

Windows 使用 `target/release/okf.exe`。本地目录模式复制语料到临时工作区；Git 模式会访问指定远端，要求解析后的提交与 checkout HEAD 一致，并模拟刷新失败和 Git 不可用。只在临时副本中操作，结束时核对源语料哈希。

`--as-of` 默认固定为 `2026-10-28T00:00:00Z`，用于可复现的时效性判定；`--timeout-ms` 指定普通 CLI 调用预算，Git 注册、刷新和故障模拟使用各自的固定预算。报告记录二进制哈希、语料哈希和上游提交。输出目录创建后，执行异常也会写入 `report.json` 并返回非零状态；`trace.jsonl` 保存已尝试的 CLI 调用、原始输出和解析结果，超时保留可获取的部分输出。参数解析或输出目录创建失败发生在报告初始化前，不生成报告。

完整语料验收手动执行，不依赖公网的失败处理回归通过 `python scripts/check_official_corpus_runner.py` 运行，并纳入 CI。

### 固定基线与上游差异

`fixtures/upstream/manifest.json` 固定三个参考项目的提交、源文件路径和 SHA-256；规范判定依据见 [检查矩阵](cli-contract.md#okf-检查矩阵)。常规交付验收仅检查本地固定语料，网络变化不会改变测试输入。

```sh
python scripts/check_upstream.py target/verification/upstream-local.json
python scripts/check_upstream.py target/verification/upstream.json --online
```

在线模式先将上游 HEAD 解析为具体提交，再对比两个不可变 tree。报告分别列出规范及样例的新增、删除、内容变化与规范变化标记。缺失规范、样例目录或不完整 tree 会失败，不能当作“无变化”。脚本不下载替换文件，也不修改固定基线。GitHub 的 `Upstream compatibility report` 工作流仅手动运行，上传同一报告。

发现差异后，按以下顺序处理：

1. 根据固定规范与新规范，区分格式要求变化、样例变化和工具策略变化。
2. 将影响消费行为的有效、无效和未知扩展字段案例加入回归；保留原文字节、字段定位和来源提交。
3. 在独立修改中更新 manifest、语料和检查矩阵，注明接受或保留差异的原因，执行完整交付验收。

报告的 `review_required` 仅表示需要分析，不代表本项目不兼容。`check_support.py` 覆盖新增、删除、修改、不完整 tree 和缺失规范的分类与拒绝行为。历史在线核对结果见 [验证记录](verification.md#最近验证)。

## 性能基线与兼容语料

本节保存历史测量及复现方法。`baseline-windows.json` 未记录源码提交和完整环境信息，不作为当前源码的性能回归结论；当前字段值检索与候选派生版本 2 的成本需要重新测量。验证范围见 [验证记录](verification.md#最近验证)。

`fixtures/upstream/manifest.json` 固定三个上游仓库的 commit、源路径与 SHA-256。共 65 份 Markdown，其中 44 份普通概念，另有目录索引等保留文件。原始许可与 NOTICE 随语料保留，`.gitattributes` 禁止 Git 改写其换行。

运行 `cargo run --release --locked -p okf-adapters --example baseline -- fixtures/upstream` 可得到 JSON 基线。若有意更新语料，运行 `python scripts/fetch-fixtures.py`；该脚本使用显式固定的提交，普通测试不联网。

基线分别使用空观测缓存、预热观测缓存、已同步正文索引。每次实时查询从三个包读取 44 份文件、139099 字节，列举 19 个目录。正文索引查询不读取知识包文件，但仍读取 SQLite；指标中的 `file_reads` 和 `read_bytes` 只统计知识包适配器 I/O。

[Windows x86_64 优化构建的基线记录](baseline-windows.json)中，三个查询的实时元数据扫描约 136–238 ms，索引全文查询约 1.3–1.9 ms。这是小语料的机制基线，查询语义、命中数量不同，不能解释为同等召回质量下的加速比。操作系统文件缓存未清空，计时仅供参考，CI 不以时间阈值判断通过。

中文和代码符号的正确性由合成端到端样例验证；这份英语上游语料上的零命中不能评估中文召回质量。真实 Agent 任务的成功率、引用正确率、调用次数和上下文披露量仍需要独立任务集评测。

### 索引读取对照

`scripts/compare_index.py BEFORE AFTER OUTPUT.json` 对比两个已构建的二进制，在同一个含 1000 份文档、32026000 字节原文的索引上各执行三轮，校验统计、原文、generation 和命中片段一致。限量检索只评估 10 份概念，用于检查无关正文的加载成本。

2026-10-26 的 Windows 优化构建对照，基线为已合并 PR #8，改造后使用统计、单文档和限量概念读取接口：

| 操作 | 平均耗时：改造前 → 后 | 采样峰值 RSS 均值：改造前 → 后 |
| --- | --- | --- |
| `index status` | 223 → 174 ms | 47.9 → 17.1 MiB |
| `read --indexed` | 225 → 155 ms | 47.8 → 14.9 MiB |
| 全文检索，扫描上限 10 | 232 → 197 ms | 47.9 → 17.4 MiB |

各次调用包含进程启动，按先基线后改造版的顺序运行；不清空操作系统缓存，RSS 每 2 ms 采样。测量期间还运行了交付与 HTTPS 测试，耗时受并发负载影响。这是指定语料上的机制对照，不作通用性能保证。索引统计仍需 SQLite 聚合查询；关系导航仍加载有界完整快照。任务召回和规模测试见 [检索任务评测](#检索任务评测)。

## 检索任务评测

`fixtures/evaluation/tasks.json` 固定八份文档与十个任务，覆盖中文、英文、代码符号、Unicode 大小写展开、跨文档证据、代码块、章节边界及无答案查询。每个正例给出预期文件和证据文本；脚本校验返回片段的原文字节范围与文件哈希。

```sh
python -m pip install -r scripts/requirements-test.txt
python scripts/evaluate.py target/release/okf target/verification/retrieval.json
python scripts/evaluate.py target/release/okf target/verification/retrieval-1000.json --scale 1000
```

Windows 使用 `target/release/okf.exe`。`--scale` 可加入 0–10000 份约 1 KiB 的无关文档，不修改任务文件。生成语料、绑定和索引都位于独立临时目录。

三种消费方式使用相同查询、前五项结果、16 KiB 条目预算与 100000 扫描预算：元数据搜索后逐文件读取、全文片段、正文上下文。统计指标为：

- 正例任务的 Recall@5 与实际披露文本中的证据召回率；无答案任务单独统计正确率。
- 每任务 CLI 调用次数与披露字节。元数据流程计入后续读取，索引流程直接使用片段；披露字节沿用条目预算口径，包含卡片和定位字段。
- 总调用延迟，包含进程启动；CLI RSS 每 2 ms 采样，记录各调用中的最大值。短暂内存峰值可能被漏采，不将采样值视为严格内存上限。

当前任务集要求全文与上下文模式完整命中预期证据，并验证全部返回引用。CI 检查这些确定性条件，不以机器相关的延迟和内存阈值判定通过。

以下为 PR #9 阶段于 2026-10-26 测得的 Windows 优化构建基线，加入 1000、5000、10000 份干扰文档。全文与上下文模式的 Recall@5、证据召回率和无答案任务正确率均为 1.0，两种模式的每任务平均披露量分别为 705.7 和 719.1 字节。

| 干扰文档数 | 全文 / 上下文平均耗时 | 两种模式采样峰值 RSS 的任务均值 | 同步耗时 |
| --- | --- | --- | --- |
| 1000 | 191 / 186 ms | 约 18.1 MiB | 4.9 s |
| 5000 | 202 / 212 ms | 约 23.4 MiB | 23.4 s |
| 10000 | 228 / 224 ms | 约 29.7 MiB | 42.5 s |

规模测试使用 `--modes full_text context`；默认运行仍包含元数据检索。报告的 `preparation` 保存同步、索引统计与单文档读取的耗时和 RSS。每个 CLI 调用均新建进程，操作系统文件缓存未清空，数据不代表冷磁盘性能。

```sh
python scripts/evaluate.py target/release/okf target/verification/retrieval-10000.json --scale 10000 --modes full_text context
```

运行明细输出到指定 JSON。上述历史基线测量期间并行运行了交付与 HTTPS 测试；同阶段串行复验 1000 份干扰文档时，全文与上下文的证据召回率仍为 1.0，平均耗时约 128 / 127 ms，采样峰值 RSS 的任务均值均约 18.1 MiB。召回率仅代表这十个固定任务；不同运行负载下的延迟差异不代表代码优化收益。最新回归范围见 [验证记录](verification.md#最近验证)。

这些任务由人工定义证据，不调用模型，也不评判自然语言答案。它们验证固定任务上的检索与引用机制；真实 Agent 答案质量需要独立评测，后续计划见 [实施计划](architecture.md#2-检索规模与任务质量)。

### 跨知识包对比

`fixtures/evaluation/multi-bundle.json` 固定四个知识包、五个任务，覆盖同名概念的冲突证据、不同包的相同正文、已过期内容、审核层级和缺失索引。固定评估时间，正文同步后移走源目录，验证离线片段、哈希、字节范围及每个包独立的 generation。

```sh
python scripts/evaluate_multi_bundle.py target/release/okf target/verification/multi-bundle.json
```

每个任务分别运行逐包调用和一次多包查询，全文与正文上下文各五个任务，共 20 个任务/模式/策略组合。校验两种策略返回相同证据、来源、哈希、字节范围与 generation；缺失索引单独记录，零命中不代表全库没有答案。报告包括调用数、披露字节、延迟与采样 RSS。逐包策略每次调用的预算独立，多包策略整次请求共享预算，报告明确标注这一差异；该小语料比较不证明紧预算下两者召回相同。

### 有限预算与不均衡知识包

`scripts/evaluate_budgets.py BINARY OUTPUT.json` 生成 60、8、3 份文档的三个包并在同步后移走源目录。四个任务覆盖路径末尾证据、筛选后的稀疏证据、重复正文的来源竞争和无答案查询。两种检索模式组合扫描预算 1/3/10/100、结果上限 1/5、字节预算 512/4096，共 128 个场景，并逐一反转包选择顺序检查确定性。

完整扫描基线必须找到全部预期证据。有限预算只要求预算、版本、哈希、原文字节和覆盖声明正确，召回损失作为测量结果记录；未完成扫描的零命中不能认定无答案。报告包括证据召回、来源覆盖、披露量、扫描量、耗时和采样 RSS，CI 保存为 `budgets.json`。

2026-10-26 Windows 实测，在结果上限 5、字节预算 4096 时，两种模式的路径末尾证据召回率随扫描预算 1/3/10/100 为 0/0/0.3333/1；重复正文来源的召回率为 0.3333/1/1/1。原因是按路径读取配额内的前缀，平均分配扫描份额不能保证读到大包末尾。本轮保留当前策略为基线；后续索引候选筛选优化需要在相同预算下报告收益与退化，不能以放宽预算充当策略改进。

### 目录范围与候选索引实验

生产 CLI 的 `--dir` 先缩小目录范围，再分配索引扫描份额。`check_scope.py` 验证四种查询模式、目录边界、Unicode 与字面通配符、多包份额、离线读取及范围外损坏隔离。范围内零命中不能证明包内其他目录没有证据。

```sh
python scripts/evaluate_candidates.py target/verification/candidates.json --scale 1000
```

候选实验使用独立临时 SQLite 数据库，保存原文、哈希、归一化文本、标签列与字符三元组倒排表，不修改生产 `content.sqlite`。24 个案例覆盖末尾证据、子串、代码符号、版本号、中文、Unicode 小写展开、短词和无命中；完整检查必须与直接子串扫描一致。标签索引验证元数据先行选择，短于三个字符的查询回退有界路径扫描。

对比在新进程中打开持久数据库，报告包含进程启动的查询耗时、构建耗时、数据库体积、正文检查数与倒排条目读取数。倒排读取有独立上限，达到上限时保守报告不完整；路径扫描额外读取一条路径记录来判断覆盖是否完整，相同正文预算不代表相同总工作量。查询拒绝空白词和负数预算，允许零预算，并由 `check_support.py` 验证这些边界。

该脚本保留为独立算法实验；实际 CLI 的候选实现及完整性、兼容和修复行为由下述生产对照验证。

### 生产候选检索对照

```sh
python scripts/evaluate_index_candidates.py target/release/okf target/verification/index-candidates.json
python scripts/evaluate_index_candidates.py target/release/okf target/verification/index-candidates-1000.json --scale 1000
```

脚本使用实际 CLI 比较路径扫描与 `--candidate-budget`，默认生成 100 份干扰文档及一份末尾证据。两种正文模式各 16 个查询，核对完整条目、总数、原文哈希及字节范围，包含元数据与正文混合、多词 Unicode、短长词组合以及部分词无命中；另测目录、类型、标签、审核与有效期筛选、共享多包预算、零预算、派生数据损坏、同 generation 修复和旧索引读取。报告同步耗时、数据库体积、倒排条目数以及两种策略的正文扫描、候选行读取和查询耗时。

默认验收集加入交付入口，规模结果用于测量，不设置机器相关的耗时门槛。候选维护增加同步、数据库存储与清单检查成本，正文与候选预算必须一并报告。

以下是候选派生版本 2 之前的历史测量，不能直接作为当前字段值检索的体积、耗时或召回结论；复现时应记录二进制构建来源和派生版本。

2026-10-26 Windows 本地实测，1000 份干扰文档加一份末尾证据时，两种正文模式的 `late-evidence` 查询均从路径扫描的 1001 份正文降至 1 份正文及 12 条候选记录，完整结果一致。该语料同步约 16.9 秒，数据库约 30.3 MiB，包含 159192 条倒排记录。这些数字记录包含派生索引的实际成本，不代表同等总工作量下的加速，也不能外推到其他语料。

### CLI 消费任务回放

```sh
python scripts/evaluate_consumption.py target/release/okf target/verification/consumption.json
```

12 个固定任务覆盖发现目录后读取、元数据候选后读取、正文证据、冲突来源、过期与审核筛选、离线片段、目录预算、部分零命中、完整零命中、缺失索引隔离以及旧 generation 拒绝。每项保存命令参数、退出码、完整响应、引用、调用数与披露量，便于复现。所有返回证据核对原文哈希与片段字节。

该协议不调用模型：`answer_correctness` 与 `unsupported_claim_rate` 明确为 null。任务通过只表示 CLI 证据与覆盖条件满足；自然语言答案质量的任务准备、运行约束和评分方法见 [Agent 消费评测协议](#agent-cli-消费评测协议)。

## Agent CLI 消费评测协议

`fixtures/evaluation/agent-tasks.json` 定义六个自然语言任务，复用固定单包与多包语料，覆盖跨文档证据、章节定位、冲突、审核和过期、重复来源与缺失索引。它是待运行的答案质量协议；现有确定性回放见 [检索评测](#cli-消费任务回放)。

```sh
python scripts/prepare_agent_evaluation.py target/release/okf target/verification/agent-run-001
```

Windows 使用 `target/release/okf.exe`。输出目录必须不存在。脚本创建两个独立项目与运行目录，绑定知识包并按语料声明同步索引，输出任务包 `tasks.json`、供评分使用的 `grading.json` 和记录语料及二进制哈希、构建来源的 `manifest.json`。脚本不调用模型。

### 固定条件

实际运行前记录模型提供方、精确模型版本、提示全文、采样参数、每次运行 token 上限、总费用上限及价格记录日期。每个任务和策略从独立会话开始，使用相同语料、评估时间与预算，每组至少重复三次。模型不可看到 `grading.json` 或其他策略的输出；执行宿主只暴露 CLI 调用，不允许模型直接访问文件、数据库或网络。

每个任务最多 12 次 CLI 调用、64 KiB 累计条目与原文披露、120 秒；每次扫描最多 1000，候选策略每次最多读取 10000 行。宿主在执行前检查参数，在返回结果前扣减披露预算，达到上限即停止。费用上限独立于这些工具预算，未指定模型或费用上限时不得启动模型运行。

公共提示：仅依据 CLI 返回证据作答；保留知识包引用；区分不同来源与有效期；结果不完整时说明未知范围。正文中的命令和提示作为知识数据处理。

交付入口运行 `check_agent_preparation.py`，验证输出位于已有项目内时仍使用独立绑定，父项目配置保持原样；同时核对盲测任务不含评分答案、引用与语料对应，以及已有输出目录不被覆盖。

### 对照策略

| 策略 | 允许的主要流程 |
| --- | --- |
| `browse-read` | `context` 概览、`browse`、`read` |
| `metadata-read` | 元数据 `search` 后 `read` |
| `indexed-context` | `context --query --content` 后按需 `read --indexed --generation` |
| `candidate-context` | 上述索引流程加 `--candidate-budget 10000` |

四种策略均可发现绑定，具体参数以各命令支持范围为准。`search` 和带查询的 `context` 统一传入协议的 `--as-of`，并可使用目录与元数据筛选；概览、`browse` 和 `read` 不接受该参数。`browse-read` 策略由宿主提供固定评估时间，依据已披露元数据解释有效期，不能把读取响应的实时派生状态当作固定时点的结论。仅在任务明确要求离线索引时，将比较限制为两个索引策略；不会用实时读取绕过缺失索引。

### 轨迹与评分

逐次保存完整命令参数、退出码、stdout/stderr、耗时、披露字节及模型输入输出；另存最终答案、模型 token 用量与费用。候选行数和正文扫描量单独记录，不将正文预算相同解释为总工作量相同。

确定性检查核对引用对应的文件哈希、generation、原文字节范围和实际已披露证据。人工评分依据 `grading.json` 的必要事实和结论类型，逐项标记正确、遗漏、错误；将答案拆分为可验证断言，标记其证据是否支持。分别报告任务正确率、必要事实覆盖率、引用支持率和无依据断言比例，保留分母与单项记录。空答案单独记为拒答，不因没有错误断言计作任务成功。

未完成模型运行和评分前，答案质量指标保持未测量；脚本准备成功或确定性检索通过均不构成模型效果结论。

## 最近验证

### 2026-11-04：候选交集顺序与文档物化预算

`candidates-v1` 原先按三元组的字典序读取倒排表，并把 `--candidate-budget` 全部用于求交。当查询同时包含宽泛词元与选择性词元时，宽泛词元先耗尽预算，文档物化阶段读到 `rows_read == row_limit` 立即退出，返回零条目并标记不完整；全量扫描在同一语料上能返回全部命中。改动按倒排表长度升序求交，并为文档物化保留 `row_limit/16`（上限 1024 行）的行预算。

Windows MSVC Release 构建，301 份合成文档（`consistency` 覆盖全部文档，`throughput` 覆盖七分之一），`--max-scan 20000`、默认条目与正文预算：

| 查询 | 候选预算 | 改动前条目 / 行读取 | 改动后条目 / 行读取 |
| --- | --- | --- | --- |
| `throughput consistency` | 500 | 0 / 500 | 18 / 487 |
| `throughput consistency` | 2000 | 0 / 2000 | 5 / 1880 |
| `throughput consistency` | 3000 | 0 / 3000 | 10 / 2823 |
| `throughput consistency` | 4000 | 20 / 3087，总数 43 | 20 / 3087，总数 43 |
| `consistency` | 500 | 0 / 500 | 20 / 500 |
| `consistency` | 2000 | 0 / 2000 | 20 / 1950 |
| `consistency` | 3000 | 20 / 3000，总数 300 | 20 / 3000，总数 300 |

预算足够读完所有倒排表时，改动前后的 `items` 与 `page.total` 逐字段相同，全量扫描、改动前后候选三者一致。截断时条目数不随预算单调增加，原因与 [有限预算与不均衡知识包](#有限预算与不均衡知识包) 记录的路径前缀配额相同；两种情况下返回条目都属于完整答案的子集，并且都报告 `CANDIDATE_LIMIT` 与未知总数。

`cargo test --workspace --all-targets --locked` 123 项通过，`clippy -D warnings` 与 `fmt --check` 通过；新增 `crates/okf-adapters/tests/retrieval_parity.rs` 四项，其中「宽泛词元不占满物化预算」在改动前的实现上失败。本轮未运行 `scripts/evaluate_index_candidates.py`：本机无法访问 PyPI，`scripts/requirements-test.txt` 的依赖未安装，发布前需补跑该入口。

### 2026-11-03：冗余清理回归

合并标量筛选条件、Skill 文件清单及归档校验逻辑，压缩重复设计说明和专项流水。Windows GNU 全工作区 Debug 119 项、Clippy、fmt、Release 构建、21 项 Python 辅助测试、32 组候选检索对照及解压包 65 份文档/184 次 CLI 调用通过；归档重复条目与校验文件名拒绝新增普通、`-O` 双模式检查。文档本地链接/锚点检查通过。日志与源码哈希见 `target/verification/cleanup-20261103/`；本轮未重跑 Release 单元测试、HTTPS/LFS、跨平台或模型评测。

### 2026-11-03：当前工作区完整验证

对包含未提交代码、文档合并、Skill 按需参考及分发链路的当前工作区重新执行完整本地验证。开始时记录 169 份源码/资源哈希，并逐文件核对临时构建副本一致；执行结束再次核对，期间无源码变化。使用 Windows GNU 工具链与既有临时链接包装脚本，仓库构建配置未改动。

| 检查 | 本轮结果 |
| --- | --- |
| Rust | Debug、Release 全工作区全目标各 119 项通过；全工作区全目标 Clippy `-D warnings`、fmt 通过 |
| Python 与文档 | 21 项辅助测试、31 份脚本语法、8 份文档的 180 个本地链接/锚点、3 个工作流 YAML 通过 |
| Release 完整交付 | `check_delivery.py` 全链路通过；29 类响应 Schema、28 次索引检查/修复调用、四种模式目录范围检查通过 |
| 检索 | 默认规模及 1000 份干扰文档分别通过 32 组生产候选对照，并验证筛选、预算、多包、损坏、修复和旧库；独立原型 24 例通过 |
| 多包与消费 | 20 组多包对照、128 个预算案例及反序检查、12 个消费任务回放、Agent 隔离准备检查通过 |
| 真实 Git/LFS | 本地 HTTPS 服务上的认证隔离、刷新、压缩、GC/子模块读取和传输/配额故障通过；LFS 下载配额及真实 CLI 完整性、缺失、恢复通过 |
| 分发 | 新 Release CLI ZIP 的校验和、内嵌入口/参考、解压独立运行通过；65 份上游文档、184 次 CLI 调用通过；8 项 Skill 组装测试含参考缺失/过时拒绝 |
| 安装边界 | 已有入口/参考文件与同名目录防覆盖、无关文件保留、运行状态隔离通过；真实 Windows Junction 被拒绝，外部目录未写入 |
| 升级与回退 | 本地旧构建到当前构建的绑定、离线 generation、原文和回退通过；额外实际生成 v1 候选数据，验证新版拒绝旧候选、普通读取可用、重同步到 v2 后 generation 不变及旧程序原文读取 |
| 入门流程 | 隔离项目中的 README 流程及真实 PowerShell 引用示例通过 |

本轮未发现新的产品回归。额外迁移探针首次因 Python 测试代码未关闭 SQLite 连接而在临时目录清理时失败，修正探针并重跑通过，未修改产品代码。完整日志、源码清单、二进制哈希和机器可读摘要保存在 `target/verification/workspace-20261103-151638/`；本节属于执行完成后追加的记录。

旧构建是本地 `0.1.0` GNU 构建，不据此承诺任意版本兼容。此前 9 次独立 Agent 实测及评分缺口保留在下方，本轮未重复调用模型。本地验证不替代 Linux/macOS、Windows MSVC 正式发行、真实 SSH 服务、最低系统版本或三平台正式 Skill ZIP 验收。


### 2026-11-03：独立 Agent 按需消费实测

本会话实际启动 9 个不继承审查对话的独立 Agent：6 个固定自然语言任务、2 个修正后复测及1个补充校验任务。每个 Agent 仅获任务、已安装 Skill 和 CLI 宿主入口，不提供预期答案；各任务使用隔离项目与运行目录，缺失索引任务还移走实时源目录。宿主记录原始 CLI 参数、stdout、stderr、退出码及参考章节读取，限制每任务 12 次 CLI 调用、64 KiB CLI 披露和 300 秒。模型继承当前会话，工具未暴露精确模型版本和采样参数。

| 场景 | CLI 调用数 | 参数错误数 | 按需读取的参考章节 |
| --- | --- | --- | --- |
| 认证与重试要求 | 5 | 0 | 未加载参考 |
| 章节恢复步骤 | 9 | 1 | indexed-and-offline-workflows |
| 冲突来源 | 6 | 0 | 未加载参考 |
| 固定日期与审核筛选 | 6 | 0 | retrieval-and-filters |
| 相同内容的独立来源 | 10 | 2 | 未加载参考 |
| 离线缺失索引 | 5 | 0 | indexed-and-offline-workflows |
| 章节恢复步骤（修正后） | 6 | 0 | indexed-and-offline-workflows |
| 独立来源（修正后） | 8 | 0 | 未加载参考 |
| 附件、锚点与零警告门槛 | 5 | 0 | validation |

所有运行均未请求参考全文。普通查阅任务只用入口；章节定位与离线任务按需读取索引章节，生命周期任务读取筛选章节，补充校验任务读取校验章节。7 种任务均给出了符合用户提问的答案，保留来源冲突、缓存限制和未知覆盖；原始 6 任务按预置评分表严格计为 5/6 项完整、11/12 个必要事实。未覆盖项是生命周期回答未额外说明 legacy 已过期；该事实未在用户提问中明确要求，仍保留原评分差异，不修改基准或补算通过。

首次章节与重复来源任务分别发生 1 次、2 次参数错误：PowerShell 将未加引号的 `@book` 当作 splatting，导致引用参数丢失。Agent 自行加引号后完成任务。已将 Skill 示例统一改为带引号的引用并补充说明；重新构建、安装后由两个全新 Agent 复测，均无参数错误。调用次数下降只是本轮观察，不能归因为统计显著的性能提升。

补充校验正确报告缺失 PDF 附件与缺失锚点，区分结构失败数 0 与质量门槛失败（2 警告、退出码 1）。共复核 15 处返回原文/片段的哈希与字节范围，均与测试语料一致。修正后 Skill 格式校验和新二进制独立冒烟通过。

原始调用轨迹、实际安装的两版 Skill、二进制哈希、任务与评分摘要位于 `target/verification/skill-agent-smoke/`，完整 Agent 回答保留于本会话。该目录为本地生成产物。测试按合作式宿主使用约束运行，不是文件系统安全隔离验收；没有限制为固定策略组，也未按正式 `agent-cli-v1` 协议做三次重复、模型费用与 token 统计。因此这是实际行为冒烟及一次修复复测，不能作为通用答案正确率或全平台兼容性结论。


### 前期专项验证

以下专项已被 2026-11-03 完整工作区回归覆盖，保留独立证据入口；当前覆盖范围以上方完整验证及 Agent 实测为准。

| 日期与专项 | 独立证据或补充范围 |
| --- | --- |
| 2026-11-03 功能文档核对 | 21 个命令帮助页、40 个长选项与实际参数边界探针；`target/verification/document-features.json` |
| 2026-11-03 一致性修复 | 元数据全局排序与字段值检索、目录 Junction、HTML 锚点、凭据路径和子模块配额；`target/verification/review-fixes/` |
| 2026-11-03 Skill 分发 | 入口与参考安装、防覆盖、组装拒绝与解压验收；实际按需读取结果见上方 Agent 实测 |
| 2026-11-03 Git/LFS | 本地 TLS 认证隔离、GC、子模块及 LFS 损坏/恢复；脚本入口见 [验证入口](#验证入口) |
| 2026-11-02 格式检查 | 附件类型报告、HTML 锚点字符引用回归及 Schema 检查通过 |
| 2026-10-31 格式检查 | 图片引用、显式 HTML 锚点、外部链接计数回归及 Schema 检查通过 |
| 2026-10-27 Skill 更名 | 当时 Debug/Release 各 94 项及完整交付验收通过；`target/verification/skill-rename/`。标签重建与旧名称迁移见 [发行说明](releases.md#skill-更名与升级) |

这些本地记录使用 Windows GNU；临时链接包装脚本用于绕过本机链接阻塞，未更改仓库构建配置。生成产物不纳入版本控制，历史测试数量不代表当前覆盖。

### 2026-10-26：候选检索与发行基线

2026-10-26 在 Windows 上验证候选检索、索引完整性、目录范围、消费任务及有限预算评测：

| 检查 | 结果 |
| --- | --- |
| Rust 测试与静态检查 | Windows Debug、Release 各 94 项通过；fmt、Clippy 通过；包含缓存锁重复句柄释放回归；三平台结果以 PR CI 为准 |
| 响应契约 | 29 类实际 CLI 响应通过 JSON Schema 校验 |
| 目录范围 | 实时搜索、查询上下文及两种索引模式通过；覆盖目录边界、Unicode、字面通配符、多包扫描份额及离线范围外损坏隔离 |
| 消费任务 | 12 个确定性任务回放通过，保存完整命令轨迹；未测量模型答案正确率 |
| 候选检索 | 100 和 1000 份干扰文档两个规模分别通过 32 个完整结果对照（每种正文模式 16 个）；另覆盖筛选、共享预算、损坏修复和旧库；成本见检索评测 |
| Agent 评测准备 | 六个盲测任务及两个隔离 CLI 项目实际准备成功；父项目配置不变、引用与语料哈希、盲测内容及拒绝覆盖回归通过；尚未调用模型或评分 |
| 辅助检查 | 三个上游在线比较无差异；Windows GNU/MSVC 导入表检查通过，8 项依赖门槛、上游分类及候选实验参数与零预算回归通过 |
| HTTPS Git | 真实 Git 经本地 HTTPS 服务完成获取、刷新、压缩及证书、认证、404、断连、超时、配额故障测试 |
| 检索 | 本轮固定任务、1000 份干扰文档全模式、跨包 20 个任务/模式/策略组合及 128 个有限预算案例通过；5000/10000 规模结果保留为此前基线 |
| 发行拒绝条件 | 8 类场景在普通与 `-O` 模式下均通过；覆盖范围见 [发行验收](releases.md#本地发行验收) |
| 独立运行 | 解压包通过 smoke 测试；65 份固定上游文档的实时与离线验收共执行 184 次 CLI 调用，包含索引正文损坏后的同步修复 |
| 发行演练 | `c064e39` 的固定三平台打包运行 [40292399047](https://github.com/coolplayagent/okf-cli/actions/runs/40292399047) 通过；下载产物哈希及 Windows 解压运行已复核；未创建 tag、草稿或版本 |
| 索引完整性 | 28 次实际 CLI 调用覆盖缺失索引、扫描及累计 64 MiB 上限、部分扫描中的正文或派生损坏、版本不兼容、离线检查与同步修复；只读检查前后数据库内容一致 |
| 升级与回退 | Windows 下载包升级至本轮本地构建再回退，绑定、generation 和离线读取保持一致；覆盖候选发布、旧程序读取扩展库、旧程序写入使候选失效及重同步修复 |
| 文档与配置 | 本地文档链接、工作流 YAML、Python 语法、消费 Skill 和差异格式检查通过 |

本地全量回归、千份文档候选对照、升级回退和最终解压包验收日志保存在 `target/verification/quality/`；该目录是生成产物，不纳入版本控制。

索引回归覆盖事务失败回滚、损坏检测和同 generation 修复；命令语义见 [索引完整性检查](cli-contract.md#索引完整性检查)。有限预算评测记录当前检索策略的召回损失，指标及适用边界见 [检索任务评测](#检索任务评测)。

PR #11 的三平台 CI 已通过并合并为 `c064e39`。后续 PR #12 已合并，`v0.1.0` 在提交 `720bafc` 完成三平台发行验收并发布；2026-10-27 更新后的 `v0.1.0` 标签及重建附件已包含 Skill 更名。此前下载的同版本附件可能仍使用旧名称。产物和运行库要求见 [发行说明](releases.md)。

HTTPS 阶段日志保存到 `target/verification/https`，Unix 另保留临时本地仓库的 Git 事件和传输跟踪；两个工作流均上传这些产物。可用 `python scripts/check_git_https.py BINARY --repeat 5` 连续执行，或加 `--payload-bytes 1048576` 扩大传输载荷。任一轮失败立即停止。配额测试断言没有遗留 Git 锁，并验证后续刷新成功。
