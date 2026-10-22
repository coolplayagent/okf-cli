# CLI 与 Agent 参考

本页描述可执行命令和响应语义。二进制名称为 `okf`，发行包名称为 `okf-cli`。安装和入门见 [README](../README.md)，后续功能见 [实施计划](roadmap.md)。

## 命令

| 命令 | 作用 |
| --- | --- |
| `bundle add NAME --path PATH [--kind fs]` | 检查目录并保存知识包绑定 |
| `bundle list` / `bundle show NAME` | 查看知识包绑定配置 |
| `bundle remove NAME` | 解除绑定，保留知识包文件 |
| `context [--bundle NAME]` | 返回知识包概览与目录入口 |
| `context --query QUERY [--bundle NAME]` | 返回元数据候选与原文读取入口 |
| `browse REF` | 展开一层目录 |
| `read REF [--raw]` | 返回原始 UTF-8 文件的一个分页 |
| `search QUERY [--bundle NAME]` | 检索 ID、标题、摘要、标签和类型 |
| `status [NAME]` | 查看已观测元数据的缓存统计 |
| `validate NAME` | 检查知识包中的概念与保留文件 |
| `doctor` | 查看版本、运行目录、项目目录、知识包数量和缓存可用性 |
| `skill install PATH` | 将内置消费 Skill 写入指定目录 |
| `help [COMMAND]` / `--help` | 显示文本帮助 |

`--bundle` 可重复使用，适用于概览和检索。未指定时选择已绑定的全部知识包。同名项目与用户知识包绑定必须通过 `--scope` 区分。`context` 和 `search` 没有知识包时返回 `NO_BUNDLES`，不会自动扫描当前目录寻找知识包。

别名长度为 1–64 个 ASCII 字符，只能包含字母、数字、`-` 和 `_`。存储类型支持 `fs`，其他类型返回 `UNSUPPORTED_CAPABILITY`。

知识包、概念、来源与绑定别名的定义见 [领域术语](architecture.md#领域术语)。配置中的 `bundles` 保存绑定，响应中的同名字段保存读取观测；概念的 `sources` 专指依据材料。

## 全局选项

| 选项 | 默认值 | 含义 |
| --- | --- | --- |
| `--format FORMAT` | `text` | `text` 为缩进 JSON，`json` 为紧凑 JSON |
| `--scope SCOPE` | 读取两个作用域；新增绑定写入项目作用域 | 使用 `project` 或 `user` 限定配置知识包 |
| `--home PATH` | `OKF_HOME`，否则系统本地数据目录下的 `okf-cli` | 指定绝对运行目录 |
| `--max-output-bytes N` | 131072 | 整个序列化响应的字节上限；允许 1024–1048576 |

业务响应的 stdout 是单个 JSON 对象，不混入日志。帮助和版本输出是文本，包括使用 `--format json` 时。参数错误在显式请求 JSON 时使用相同响应 envelope。

配置位置与知识包绑定方法见 [README](../README.md#配置)。

## 引用与原文

`browse @handbook` 展开知识包根目录，`browse @handbook/services` 展开子目录。`read @handbook/services/auth` 读取 `services/auth.md`，也可以传入完整文件名。以 `.md` 结尾的读取引用按完整文件名处理。

`browse` 的目录索引使用 `index.origin` 区分 `authored` 与 `synthesized`；目录未完整列举且尚未发现索引时为 `unknown`。作者索引通过 `index.ref` 单独读取，不嵌入目录结果。`index.md` 与 `log.md` 可直接传给 `read`。

普通概念的 frontmatter 必须包含非空字符串 `type`。`read --raw` 可查看不合规 Markdown，并附解析诊断；无效 UTF-8 和大小限制仍会阻止读取。`validate` 还检查根索引 frontmatter 的允许字段、子目录索引限制及日志日期标题。

结构、元数据与正文诊断规则见 [OKF 检查矩阵](okf-validation.md)。质量警告保留概念与原文；缺少可选字段、未知类型或扩展字段不触发警告。

`read` 在 `data.document` 中返回 `bundle_id`、`path`、`concept_id`、`content_sha256`、`byte_start`、`byte_end`、`total_bytes` 和 `text`。字节范围为完整原始文件中的半开区间，包括 BOM、frontmatter 和换行。拼接同一修订的分页 `text` 可以还原文件。

## 预算与分页

各命令只接受下表列出的选项，不适用的选项返回参数错误。单元格给出默认值，`—` 表示不接受该选项。

| 命令 | `--limit` | `--budget-bytes` | `--max-scan` | `--cursor` | `--timeout-ms` |
| --- | --- | --- | --- | --- | --- |
| `context` 概览 | 20 | 24576 | — | — | 30000 |
| `context --query` | 20 | 24576 | 100 | — | 30000 |
| `browse` | 20 | 16384 | 100 | 可选 | 30000 |
| `read` | — | 65536 | — | 可选 | 30000 |
| `search` | 20 | 16384 | 100 | — | 30000 |
| `validate` | — | — | 100 | — | 30000 |

取值范围：`--limit` 为 1–100，`--budget-bytes` 为 1–65536，`--max-scan` 为 1–10000，`--timeout-ms` 为 1–3600000 毫秒。

原文预算计算返回文本的 UTF-8 字节数，条目预算计算序列化条目字节数；完整响应另外受 `--max-output-bytes` 限制。原文按 Unicode 字符边界分页，放不下下一个字符时返回 `BUDGET_TOO_SMALL`。浏览放不下首个条目时同样报错；搜索跳过超出预算的卡片并报告 `CARD_TOO_LARGE`，概览预算不足时报告 `BUDGET_EXCEEDED`。

扫描预算在文件系统适配器内扣减：打开一个目录计一次，每次推进目录迭代器计一次，包括非 Markdown 文件与确认目录结束的那一步。所有目录共用本次请求的预算，不会先列举整个目录再截断。`budget.scan_limit` 和 `budget.scanned_entries` 返回上限与实际计数。单文件读取上限为 2 MiB。

目录清单按路径排序，分页 cursor 校验完整清单的修订。列举预算不足时，`browse` 返回已观测条目的部分结果、`SCAN_LIMIT` 和退出码 7，不生成 cursor；应提高 `--max-scan` 后重新浏览。带 cursor 的请求无法在预算内重建完整清单时返回 `SCAN_LIMIT` 错误，不使用不完整清单验证修订。

有后续分页时，`page.has_more` 为 true，`page.next_cursor` 和 `next` 给出继续入口。将 cursor 原样传给相同知识包、目标和操作；遇到 `REVISION_CHANGED` 时重新开始，不拼接不同修订。正常目录和原文分页返回退出码 0；搜索没有续页，达到结果或扫描上限时应缩小范围或提高预算。

## 截止时间与取消

消费命令支持 Ctrl+C 取消，诊断为 `CANCELLED`；超过截止时间时为 `DEADLINE_EXCEEDED`，均使用退出码 7。检查点覆盖目录遍历、逐文件处理和分块读取。概览、检索和校验保留已完成结果；浏览在处理条目时中断也保留已有结果，但无法完成初始目录列举时返回错误。原文读取中断返回错误。

截止时间从请求执行开始计时，在检查点协作生效；操作系统中已阻塞的单次 I/O、数据库操作与序列化不能被强制中断。这个选项不能作为进程运行时间的硬上限。

## 搜索与生命周期

查询按空白拆词，忽略大小写，并要求所有词都在卡片的检索字段中出现。支持中文子串，结果按知识包引用排序。`context --query` 复用元数据检索，返回 `disclosure: metadata_candidates` 与 `read` 入口。

卡片中的 `trust`、`status` 和 `expiry` 从元数据派生。审核层级为 `unverified`、`machine-confirmed` 或 `human-reviewed`；过期状态为 `unspecified`、`current`、`expired` 或 `unknown`。默认值与异常值的处理见 [派生与覆盖](okf-validation.md#派生与覆盖)。这些字段表达源文件声明，不提供身份验证或内容真实性保证。

## 故障与覆盖

浏览或检索遇到坏概念、文件读取失败时，返回带路径的诊断和部分结果，继续处理可用条目。多知识包概览和检索隔离知识包故障，保留其他包的结果；单知识包初始目录不可读时返回错误。未开始读取、探测失败或未能展开目录的包使用 `freshness: unknown`、`checked_at: null` 与未知覆盖状态。

除 Skill 安装外，业务命令会尝试打开 SQLite 缓存。消费命令遇到缓存故障时保留可读内容：写入失败使用 `CACHE_WRITE_FAILED`，打开失败且本次操作没有尝试写入卡片时使用 `CACHE_UNAVAILABLE`。配置管理、`status` 和 `doctor` 保留缓存错误。

缓存问题与知识文件覆盖分别表达，完整校验发现的文档错误优先于缓存诊断。`SCAN_LIMIT` 仅用于扫描预算耗尽，文件故障或取消不会被标为预算不足。

`validate` 返回 `data.link_check`，其中 `scope` 为 `bundle_markdown_targets`，`complete` 为布尔值。它说明包内 Markdown 链接与目录目标检查是否完成；扫描未完成时跳过目标存在性判断，读取失败、链接预算耗尽、正文收集或警告截断，以及取消都会使其为 false。它不表示目标全部有效或关系图已建立，检查范围见 [正文链接与来源脚注](okf-validation.md#正文链接与来源脚注)。

`budget.link_limit` 与 `link_bytes_limit` 给出待检查链接的数量、字符串字节上限；`links_checked` 是实际检查的链接数，`link_bytes_used` 是保留链接占用的字符串字节数。`LINK_LIMIT` 返回退出码 7，链接阶段取消或预算耗尽均保留已完成的元数据覆盖。具体上限见 [诊断上限](okf-validation.md#诊断上限)。

## JSON 响应

完整结构由 [响应 Schema](../schemas/response.schema.json) 定义。

| 字段 | 含义 |
| --- | --- |
| `schema_version` | envelope 版本，值为 `2.0` |
| `request_id` | 本次请求的诊断标识 |
| `command` | 操作名称；通用执行错误使用 `request`，参数错误使用 `arguments` |
| `status` | `ok`、`partial` 或 `error` |
| `data` | 操作结果，或 `error.code` 与 `error.message` |
| `bundles` | 知识包读取观测、时间、视图及覆盖范围 |
| `page` | `returned`、`total`、`has_more` 和 `next_cursor` |
| `budget` | 操作对应的内容或扫描量统计 |
| `diagnostics` | 诊断 code、severity、message、path、field 和可选 location |
| `next` | 建议的后续操作、目标和可选 cursor |

读取视图为 `observed`，上游修订为 null。`metadata` 覆盖状态必须结合 `coverage.scope` 解读：单文件、单目录与全知识包扫描不是同一范围。`body`、`graph` 是保守的覆盖声明，不能解释成离线正文或关系图已经建立。

`page.total` 可以为 null；`has_more=false` 只说明该操作没有续页，不证明全库扫描完整。卡片返回也不意味着 Agent 已读取正文。

`diagnostics[].severity` 为 `error` 或 `warning`。元数据及正文警告的 `path` 为完整文件引用，如 `@handbook/services/auth.md`；元数据的 `field` 为 frontmatter 的 JSON Pointer，如 `/verified/0/at`。正文、操作或文档级诊断的 `field` 为 null，包括截断提示。警告不改变执行状态或元数据覆盖；仍应结合 `status`、退出码与检查完整性判断结果。诊断条数规则见 [诊断上限](okf-validation.md#诊断上限)。

正文诊断通过可选的 `location: {byte_start, byte_end}` 标识完整原文件中的 UTF-8 字节半开区间，包含 BOM、frontmatter 与换行所占字节；它不相对于当前读取分页。没有具体正文位置的诊断省略此字段。

## 退出码

| 退出码 | 含义 |
| --- | --- |
| 0 | 成功，包括正常分页、完整范围内零命中或过期内容 |
| 1 | 文档不合规或合规检查发现错误 |
| 2 | 参数、配置、知识包歧义或不支持的能力 |
| 3 | 知识包、目录或文件不存在 |
| 4 | 访问被拒绝、路径越界或目录链接被拒绝 |
| 5 | I/O、缓存或配置锁错误 |
| 6 | 读取期间内容变化或分页修订已变化 |
| 7 | 部分结果、资源超限、截止时间或取消 |

完整合规检查发现错误时为 `status: error`、退出码 1，缓存不可用诊断不会覆盖这个结果；校验扫描未完成时为 `partial`、退出码 7，结果中仍保留已发现的错误。文件过大和输出超限也使用退出码 7，但响应为 `error`。非法 cursor 使用退出码 2，修订变化使用退出码 6。

`validate` 返回 `data.failures` 与 `data.warnings`，分别统计失败文件与返回的元数据及正文警告。只有警告且扫描完成时仍为 `ok`、退出码 0。

Agent 应读取稳定的诊断 code，避免匹配错误文本；只引用实际读取的文件、字节范围和内容哈希，并保留影响答案的过期、审核和覆盖限定。消费流程见 [消费 Skill](../skills/okf-consume/SKILL.md)。
