# OKF 检查矩阵

本页定义消费过程中的结构检查与元数据建议，依据为固定提交的 [OKF v0.2 规范](https://github.com/GoogleCloudPlatform/open-knowledge-format/blob/ad30107c31c06aec8a7d5636e0d1058118604e6f/SPEC.md)。命令和响应规则见 [CLI 参考](cli-contract.md)。

## 诊断与消费

`read`、`browse`、`search`、`context --query` 和 `validate` 使用相同的元数据规则。

- `error` 表示结构不合规或操作故障，是否终止请求由命令决定。例如浏览跳过坏概念并返回部分结果，显式 `read --raw` 可以返回带结构错误诊断的原文。
- `warning` 表示可选元数据的形状或取值不符合规范指导。警告不拒绝概念、不改变退出码，也不降低知识包读取覆盖率。
- 可选字段缺失、未知 `type`、未知扩展字段和缺失索引不会触发元数据警告。未知字段保持原值，原文不被重写。

诊断字段定义见 [JSON 响应](cli-contract.md#json-响应)。下表中的字段级警告通过 `field` 定位，例如 `/verified/0/at`。

## 检查规则

| 检查对象 | 规则与规范依据 | 诊断 code | 级别 |
| --- | --- | --- | --- |
| 概念结构 | UTF-8、可解析的 YAML mapping、非空字符串 `type`；§4、§11 | `INVALID_OKF` | error |
| 保留文件 | 根索引允许 `okf_version` frontmatter，子目录索引不带 frontmatter；日志日期标题为有效的 `YYYY-MM-DD`；§8、§9 | `INVALID_OKF` | error |
| `title`、`description`、`resource` | 出现时为字符串；§4.1 | `INVALID_METADATA_TYPE` | warning |
| `tags` | 出现时为字符串列表，定位到无效元素；§4.1 | `INVALID_METADATA_TYPE` | warning |
| `generated` | 出现时为对象，`by` 为非空 Actor；`at` 可省略，出现时检查时间；§5.2、§7 | 类型、必填字段、Actor 或时间诊断 | warning |
| `verified` | 对象与单元素列表等价；每个事件为对象，包含 `by` 与 `at`；§5.2、§7 | 类型、必填字段、Actor 或时间诊断 | warning |
| `sources` | 出现时为对象列表，每项包含非空 `resource`；URI、包内路径与范围描述均可作为材料标识；§5.1 | `INVALID_METADATA_TYPE`、`MISSING_METADATA_FIELD`、`INVALID_METADATA_VALUE` | warning |
| 来源标识与文字字段 | `id`、`title`、`author` 出现时为字符串；`id` 非空且在概念内唯一；§5.1 | `INVALID_METADATA_TYPE`、`INVALID_METADATA_VALUE`、`DUPLICATE_SOURCE_ID` | warning |
| 来源信号 | `usage_count` 为非负整数；`last_modified` 为带时区时间；§5.1 | `INVALID_METADATA_VALUE`、`INVALID_TIMESTAMP` | warning |
| `usage_window` | 共享或来源内的窗口为对象，包含带时区的 `from`、`to`，且起点不晚于终点；§5.1 | 类型、必填字段、时间诊断及 `INVALID_TIME_RANGE` | warning |
| `status` | 出现时为字符串，规范值为 `draft`、`stable`、`deprecated`；§5.4 | `INVALID_METADATA_TYPE`、`INVALID_STATUS` | warning |
| `stale_after` | 出现时为带显式 UTC 偏移的时间；§5.5 | `INVALID_TIMESTAMP` | warning |

对象中缺少上述必填子字段使用 `MISSING_METADATA_FIELD`，必填字符串为空使用 `INVALID_METADATA_VALUE`。Actor 采用 `human:<id>`、`process:<id>` 或 `<producer>/<version>`，形状不符使用 `INVALID_ACTOR`。来源 `author` 保留生产者声明，仅检查字符串形状，不参与审核层级派生。

时间检查与过期派生共用解析器，支持 RFC 3339、无冒号的数字偏移、精确到分钟的扩展时间和基本日期时间形式；均要求显式偏移与正确的组件补零宽度。纯日期、缺少时区、非法偏移或无法表示的日期时间使用 `INVALID_TIMESTAMP`。

整数读取保留 YAML 解析器支持的 128 位值。超出 64 位范围的合法非负 `usage_count` 仍按整数处理；负数和小数报告警告，原文保持不变。

## 派生与覆盖

卡片只从有效 Actor 标识派生审核层级；无有效核验者时为 `unverified`，人类核验者优先于机器核验者。缺少或非字符串 `status` 派生为 `stable`；未知字符串状态原样保留并附警告。非法 `stale_after` 派生为 `unknown`，到期判断使用 `now >= stale_after`。

每份概念最多返回 64 项元数据发现；超出时追加一项 `METADATA_DIAGNOSTICS_TRUNCATED` 警告。这个上限仅限制诊断条目，知识包覆盖仍由实际读取范围决定；整个响应继续受输出字节上限约束。

## 范围与验证

`sources[].resource` 按材料标识检查，不会触发网络访问或文件存在性检查。尚未覆盖的 OKF 能力见 [已知边界](implementation-status.md#已知边界)。

[固定样例](../crates/okf-core/tests/fixtures/metadata-cases.json) 记录规范版本、输入元数据、预期诊断和派生结果。验证应覆盖字段形状、单对象与列表等价、时区与到期边界、原文字节不变，以及警告与结构错误、扫描中断并存。

可使用 [okf-skills 校验器](https://github.com/scaccogatto/okf-skills/blob/49f8fb4987989b7a443ce278c6aa68ba0493b51f/skills/validate/scripts/okf_validate.py) 交叉核对默认模式下的结构合规结果。参考工具会提示部分推荐字段缺失，并采用不同的时间、来源和链接建议策略；本项目的警告策略以本矩阵为准，不要求警告条数一致。
