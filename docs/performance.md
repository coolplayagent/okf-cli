# 性能基线与兼容语料

`fixtures/upstream/manifest.json` 固定三个上游仓库的 commit、源路径与 SHA-256。共 65 份 Markdown，其中 44 份普通概念，另有目录索引等保留文件。原始许可与 NOTICE 随语料保留，`.gitattributes` 禁止 Git 改写其换行。

运行 `cargo run --release --locked -p okf-adapters --example baseline -- fixtures/upstream` 可得到 JSON 基线。若有意更新语料，运行 `python scripts/fetch-fixtures.py`；该脚本使用显式固定的提交，普通测试不联网。

基线分别使用空观测缓存、预热观测缓存、已同步正文索引。每次实时查询从三个包读取 44 份文件、139099 字节，列举 19 个目录。正文索引查询不读取知识包文件，但仍读取 SQLite；指标中的 `file_reads` 和 `read_bytes` 只统计知识包适配器 I/O。

[Windows x86_64 优化构建的基线记录](baseline-windows.json)中，三个查询的实时元数据扫描约 136–238 ms，索引全文查询约 1.3–1.9 ms。这是小语料的机制基线，查询语义、命中数量不同，不能解释为同等召回质量下的加速比。操作系统文件缓存未清空，计时仅供参考，CI 不以时间阈值判断通过。

中文和代码符号的正确性由合成端到端样例验证；这份英语上游语料上的零命中不能评估中文召回质量。真实 Agent 任务的成功率、引用正确率、调用次数和上下文披露量仍需要独立任务集评测。
