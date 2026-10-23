# OKF 检查矩阵

本页定义结构检查、元数据与正文诊断，依据为固定提交的 [OKF v0.2 规范](https://github.com/GoogleCloudPlatform/open-knowledge-format/blob/ad30107c31c06aec8a7d5636e0d1058118604e6f/SPEC.md)。命令和响应规则见 [CLI 参考](cli-contract.md)。

## 诊断与消费

`read`、`browse`、`search`、`context --query` 和 `validate` 共用元数据与正文诊断规则。包内链接目标的存在性与文件类型仅由 `validate` 检查。

- `error` 表示结构不合规或操作故障，是否终止请求由命令决定。例如浏览跳过坏概念并返回部分结果，显式 `read --raw` 可以返回带结构错误诊断的原文。
- `warning` 表示元数据或正文中的质量建议，包括来源归因问题与断链。警告不拒绝概念、不改变退出码，也不降低元数据覆盖。
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

## 正文链接与来源脚注

`sources[].resource` 按材料标识检查，不会触发网络访问或文件存在性检查。来源归因通过脚注标签与 `sources[].id` 关联，不依赖来源在列表中的位置；来源无需全部被引用。

使用 Markdown 解析器提取普通链接、引用式链接与脚注，排除图片、代码和 HTML 块。包根路径以 `/` 开头，文档相对路径允许 `.` 和 `..`，归一化后必须留在知识包内。路径中的百分号编码按 UTF-8 解码；查询串和片段不参与文件定位。外部 URI 不触发访问，片段保留但不校验渲染器生成的锚点。

目录形式包括解码后以 `/` 结尾的路径，以及末段为 `.` 或 `..` 的导航。文件链接与目录链接分别匹配相应类型的目标。Markdown 脚注定义的匹配可以忽略大小写，来源 ID 则按引用标签精确关联。

| 检查对象 | 诊断 code | 行为 |
| --- | --- | --- |
| 非法路径字符或编码 | `INVALID_LINK_PATH` | 当前文件返回正文警告 |
| 归一化后越出知识包 | `LINK_PATH_ESCAPE` | 当前文件返回正文警告，不读取目标 |
| 脚注没有匹配的来源 ID | `UNRESOLVED_SOURCE` | 当前概念返回正文警告 |
| 脚注引用缺少定义 | `UNDEFINED_FOOTNOTE` | 当前概念返回正文警告 |
| 包内 Markdown 文件或显式目录不存在，或文件与目录类型不符 | `BROKEN_LINK` | 仅在 `validate` 完整列举后检查 |

按需读取仅检查当前文件，不访问链接目标；`validate` 复用完整目录清单检查目标。保留文件参与链接检查，但不要求来源 frontmatter。正文位置的响应格式见 [JSON 响应](cli-contract.md#json-响应)。

## 计算合同与正文

仅对 `type: Attested Computation` 启用以下检查，依据为规范 §10.2、§10.3。未知类型的同名扩展字段保持原样；runtime 和参数类型名称均不限定枚举。

| 检查对象 | 规则 | 诊断 |
| --- | --- | --- |
| `runtime` | 此类型需要给出非空字符串 | 通用字段缺失、类型或取值警告 |
| `parameters` | 可省略或为空列表；出现时每项为对象，包含非空字符串 `name`、`type` 和布尔值 `required` | 通用字段警告；重名使用 `DUPLICATE_PARAMETER_NAME` |
| `computation` | 出现时为非空文件路径或 URI 字符串 | 通用字段警告 |
| `executor`、`attester` | 出现时为对象，内部 `resource` 为非空字符串 | 通用字段警告 |
| `executor.receipt` | 出现时为非空字段名字符串组成的列表，可为空列表 | 通用字段警告 |
| 计算内容缺失 | 未声明 `computation` 时，需要内联计算块 | `MISSING_COMPUTATION` |
| 两种形式并存 | 已声明 `computation`，正文仍有内联计算块 | `CONFLICTING_COMPUTATION` |
| 多个内联计算块 | 计算章节内包含多个围栏代码块 | `MULTIPLE_COMPUTATIONS` |
| 空计算块 | 内联计算块仅含空白 | `EMPTY_COMPUTATION` |

内联计算使用顶层一级标题（H1）`Computation` 章节内的单个非空围栏代码块，章节持续到下一个顶层 H1。标题按 Markdown 可见文本匹配，区分大小写、忽略两端空白，包含行内格式和图片替代文本；换行保留词间边界。

子标题不会结束计算章节，其中的围栏代码块仍计入计算块数量。章节之外的示例代码，以及引用块、列表、脚注和代码中的标题不作为计算入口。缩进代码块不满足围栏代码块约定。

最小内联形式：

````markdown
---
type: Attested Computation
runtime: python
---
# Computation
```python
result = 1 + 1
```
````

使用文件形式时，在 frontmatter 中声明 `computation: calculations/total.py`，并省略内联计算块。

字段问题通过 JSON Pointer 定位，计算正文问题使用原文件字节范围。文件声明本身不合法时报告字段问题，避免再附加缺少内联计算的级联警告。所有计算诊断均为质量警告，与其他元数据和正文警告共用相应上限。

按需读取不访问声明资源。`validate` 对 `computation`、`executor.resource`、`attester.resource` 执行元信息查询，不读取资源内容、执行计算或核验 Receipt。路径相对于知识包根目录，外部 URI 跳过；`sources[].resource` 不参与此检查。

| 资源诊断 | 含义 |
| --- | --- |
| `MISSING_RESOURCE` | 包内声明文件不存在，字段级 warning |
| `RESOURCE_NOT_A_FILE` | 资源实际为目录，字段级 warning |
| `INVALID_RESOURCE_PATH` | 声明路径不能安全归一化，字段级 warning |
| `RESOURCE_LIMIT` | 资源数量或字符串预算耗尽，partial / 7 |

访问被拒绝、路径越界或适配器故障保留原操作诊断，并使资源检查覆盖不完整。资源检查与 Markdown 链接检查分别报告覆盖；目录扫描不完整时仍可报告已显式查询的资源是否存在。每份计算定义最多三个资源字段警告，与元数据/正文上限分别计数。有效静态合同不表示某次运行已通过 attestation。

## 诊断上限

| 范围 | 上限 | 超出时的诊断 | 影响 |
| --- | --- | --- | --- |
| 每份概念的元数据警告 | 64 条 | `METADATA_DIAGNOSTICS_TRUNCATED` | 追加一条截断警告，元数据覆盖保持不变 |
| 每份文件的链接与脚注引用收集 | 各 256 个 | `BODY_DIAGNOSTICS_TRUNCATED` | 追加一条截断警告，链接检查完整性为 false |
| 每份文件的正文警告 | 64 条，路径、脚注、计算正文与断链共用 | `BODY_DIAGNOSTICS_TRUNCATED` | 每份文件最多追加一条截断警告，链接检查完整性为 false |
| `validate` 保留的待检查链接 | 4096 条或 2 MiB 字符串总字节 | `LINK_LIMIT` | 返回部分结果与退出码 7，保留已完成的元数据覆盖 |

元数据和正文的警告上限分别计数；截断提示不占各自的 64 条限额。整个响应还受输出字节上限约束，`validate` 的预算字段与检查完整性见 [CLI 参考](cli-contract.md#故障与覆盖)。

## 范围与验证

扫描未完成时不推断目标缺失；文件读取失败或正文诊断截断使链接检查完整性为 false。链接检查阶段取消也保留已完成的元数据覆盖。尚未覆盖的能力见 [已知边界](implementation-status.md#已知边界)。

[固定样例](../crates/okf-core/tests/fixtures/metadata-cases.json) 记录规范版本、输入元数据、预期诊断和派生结果。元数据验证应覆盖字段形状、单对象与列表等价、时区与到期边界，以及警告与结构错误并存。

[正文测试](../crates/okf-core/tests/markdown.rs) 覆盖路径解析、引用式链接、代码排除与来源 ID 关联。CLI 验收还应检查文件与目录类型、混合警告共用上限、扫描及链接检查阶段取消，以及分页中的诊断位置始终相对于原文件。读取前后核对源文件字节与哈希。

[计算合同样例](../crates/okf-core/tests/fixtures/computation-cases.json) 覆盖合同字段、未知 runtime、零参数、内联与文件形式、章节边界、标题可见文本及空正文定位。核对各消费命令返回相同诊断，且警告不影响消费、元数据覆盖和原文字节；正文与元数据警告分别验证共享上限。

可使用 [okf-skills 校验器](https://github.com/scaccogatto/okf-skills/blob/49f8fb4987989b7a443ce278c6aa68ba0493b51f/skills/validate/scripts/okf_validate.py) 交叉核对默认模式下的结构合规结果。参考工具会提示部分推荐字段缺失，并采用不同的时间、来源和链接建议策略；本项目的警告策略以本矩阵为准，不要求警告条数一致。
