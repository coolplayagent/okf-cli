# 检索任务评测

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

2026-10-26 在 Windows 优化构建上加入 1000、5000、10000 份干扰文档，全文与上下文模式的 Recall@5、证据召回率和无答案任务正确率均为 1.0。条目新增 `bundle_id` 后，两种模式的每任务平均披露量分别为 705.7 和 719.1 字节。

| 干扰文档数 | 全文 / 上下文平均耗时 | 两种模式采样峰值 RSS 的任务均值 | 同步耗时 |
| --- | --- | --- | --- |
| 1000 | 191 / 186 ms | 约 18.1 MiB | 4.9 s |
| 5000 | 202 / 212 ms | 约 23.4 MiB | 23.4 s |
| 10000 | 228 / 224 ms | 约 29.7 MiB | 42.5 s |

规模测试使用 `--modes full_text context`；默认运行仍包含元数据检索。报告的 `preparation` 保存同步、索引统计与单文档读取的耗时和 RSS。每个 CLI 调用均新建进程，操作系统文件缓存未清空，数据不代表冷磁盘性能。

```sh
python scripts/evaluate.py target/release/okf target/verification/retrieval-10000.json --scale 10000 --modes full_text context
```

运行明细输出到指定 JSON。本次复验期间并行运行了交付与 HTTPS 测试。召回率仅代表这十个固定任务；延迟和内存受机器、并发负载、进程启动和文件缓存影响，不作为性能保证。

这些任务由人工定义证据，不调用模型，也不评判自然语言答案。它们验证固定任务上的检索与引用机制；真实 Agent 答案质量需要独立评测，后续计划见 [实施计划](roadmap.md#2-检索规模与任务质量)。

## 跨知识包对比

`fixtures/evaluation/multi-bundle.json` 固定四个知识包、五个任务，覆盖同名概念的冲突证据、不同包的相同正文、已过期内容、审核层级和缺失索引。固定评估时间，正文同步后移走源目录，验证离线片段、哈希、字节范围及每个包独立的 generation。

```sh
python scripts/evaluate_multi_bundle.py target/release/okf target/verification/multi-bundle.json
```

每个任务分别运行逐包调用和一次多包查询，全文与正文上下文各五个任务，共 20 个任务/模式/策略组合。校验两种策略返回相同证据、来源、哈希、字节范围与 generation；缺失索引单独记录，零命中不代表全库没有答案。报告包括调用数、披露字节、延迟与采样 RSS。逐包策略每次调用的预算独立，多包策略整次请求共享预算，报告明确标注这一差异；该小语料比较不证明紧预算下两者召回相同。
