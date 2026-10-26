# 上游兼容检查

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
