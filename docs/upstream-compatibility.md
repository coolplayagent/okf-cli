# 上游兼容检查

## 官方完整语料验收

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

## 固定基线与上游差异

`fixtures/upstream/manifest.json` 固定三个参考项目的提交、源文件路径和 SHA-256；规范判定依据见 [检查矩阵](okf-validation.md)。常规交付验收仅检查本地固定语料，网络变化不会改变测试输入。

```sh
python scripts/check_upstream.py target/verification/upstream-local.json
python scripts/check_upstream.py target/verification/upstream.json --online
```

在线模式先将上游 HEAD 解析为具体提交，再对比两个不可变 tree。报告分别列出规范及样例的新增、删除、内容变化与规范变化标记。缺失规范、样例目录或不完整 tree 会失败，不能当作“无变化”。脚本不下载替换文件，也不修改固定基线。GitHub 的 `Upstream compatibility report` 工作流仅手动运行，上传同一报告。

发现差异后，按以下顺序处理：

1. 根据固定规范与新规范，区分格式要求变化、样例变化和工具策略变化。
2. 将影响消费行为的有效、无效和未知扩展字段案例加入回归；保留原文字节、字段定位和来源提交。
3. 在独立修改中更新 manifest、语料和检查矩阵，注明接受或保留差异的原因，执行完整交付验收。

报告的 `review_required` 仅表示需要分析，不代表本项目不兼容。`check_support.py` 覆盖新增、删除、修改、不完整 tree 和缺失规范的分类与拒绝行为。最新在线核对结果见 [实现状态](implementation-status.md#最近验证)。
