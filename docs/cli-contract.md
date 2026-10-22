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

## 领域术语

`bundle` 管理 OKF 知识包，`--bundle` 选择已绑定的知识包。OKF 的 Source 专指概念 frontmatter `sources` 中的依据材料。完整定义见 [领域术语](architecture.md#领域术语)。

配置中的 `bundles` 保存知识包绑定；响应中的 `bundles` 保存本次读取的观测信息，`bundle_id` 标识知识包绑定。概念 frontmatter 的 `sources` 保持规范定义，不用于工具配置或读取观测。

## 全局选项

| 选项 | 默认值 | 含义 |
| --- | --- | --- |
| `--format FORMAT` | `text` | `text` 为缩进 JSON，`json` 为紧凑 JSON |
| `--scope SCOPE` | 读取两个作用域；新增绑定写入项目作用域 | 使用 `project` 或 `user` 限定配置知识包 |
| `--home PATH` | `OKF_HOME`，否则系统本地数据目录下的 `okf-cli` | 指定绝对运行目录 |
| `--max-output-bytes N` | 131072 | 整个序列化响应的字节上限；允许 1024–1048576 |

业务响应的 stdout 是单个 JSON 对象，不混入日志。帮助和版本输出是文本，包括使用 `--format json` 时。参数错误在显式请求 JSON 时使用相同响应 envelope。

配置位置与知识包绑定方法见 [README](../README.md#配置)。除 Skill 安装外，业务命令会打开运行目录中的 SQLite 缓存，因此运行目录需要可写。

## 引用与原文

`browse @handbook` 展开知识包根目录，`browse @handbook/services` 展开子目录。`read @handbook/services/auth` 读取 `services/auth.md`，也可以传入完整文件名。以 `.md` 结尾的读取引用按完整文件名处理。

`browse` 的目录索引使用 `index.origin` 区分 `authored` 与 `synthesized`。作者索引通过 `index.ref` 单独读取，不嵌入目录结果。`index.md` 与 `log.md` 可直接传给 `read`。

普通概念的 frontmatter 必须包含非空字符串 `type`。`read --raw` 可查看不合规 Markdown，并附解析诊断；无效 UTF-8 和大小限制仍会阻止读取。`validate` 还检查根索引 frontmatter 的允许字段、子目录索引限制及日志日期标题，不进行链接图或语义质量检查。

`read` 在 `data.document` 中返回 `bundle_id`、`path`、`concept_id`、`content_sha256`、`byte_start`、`byte_end`、`total_bytes` 和 `text`。字节范围为完整原始文件中的半开区间，包括 BOM、frontmatter 和换行。拼接同一修订的分页 `text` 可以还原文件。

## 预算与分页

`context`、`browse`、`read`、`search`、`validate` 的帮助列出共用的分页选项；实际生效范围如下。

| 选项 | 默认值与范围 | 生效命令 |
| --- | --- | --- |
| `--limit N` | 20；1–100 | `context`、`browse`、`search` 的返回条目数 |
| `--budget-bytes N` | 见下表；1–65536 | `context`、`browse`、`read`、`search` 的内容字节数 |
| `--max-scan N` | 100；1–10000 | `browse`、`search`、`context --query`、`validate` |
| `--cursor TOKEN` | 无 | `browse` 和 `read` 的继续读取 |

| 命令 | 默认内容预算 |
| --- | --- |
| `context`，包括任务查询 | 24 KiB |
| `browse`、`search` | 16 KiB |
| `read` | 64 KiB |

`read` 不使用条目数与扫描量选项；`validate` 仅使用扫描量选项。概览 `context` 与 `validate` 不使用 cursor，调用时应省略；`search` 和 `context --query` 不支持 cursor，会明确报错。

原文预算计算返回文本的 UTF-8 字节数；卡片预算计算序列化卡片字节数。两者均不包含完整响应 envelope；整个输出另外受 `--max-output-bytes` 限制。正文按 Unicode 字符边界分页，预算连下一个字符或卡片都放不下时返回 `BUDGET_TOO_SMALL`。

`browse` 的扫描量计数目录条目；搜索与校验同时计数目录展开和条目。单次目录列举另有 10000 条目的上限，单文件读取上限为 2 MiB。扫描预算不替代目录清单的列举上限，也没有超时或取消保证。

有后续分页时，`page.has_more` 为 true，`page.next_cursor` 和 `next` 给出继续入口。将 cursor 原样传给相同知识包、目标和操作；遇到 `REVISION_CHANGED` 时重新开始，不拼接不同修订。

正常目录或原文分页返回退出码 0。检索达到扫描、结果数量或内容预算上限时返回 `partial` 与退出码 7；不提供搜索续页，应缩小范围或提高预算。

## 搜索与生命周期

查询按空白拆词，忽略大小写，并要求所有词都在卡片的检索字段中出现。支持中文子串，结果按知识包引用排序，不提供相关性分数或正文片段。`context --query` 复用元数据检索，返回 `disclosure: metadata_candidates` 与 `read` 入口，不自动加载正文。

卡片直接返回 `trust`、`status` 和 `expiry`，没有嵌套的 `lifecycle` 字段。缺少状态时使用 `stable`；审核声明派生为 `unverified`、`machine-confirmed` 或 `human-reviewed`；过期状态为 `unspecified`、`current`、`expired` 或 `unknown`。这些字段是源元数据的派生结果，不提供身份验证或内容真实性保证。

坏概念在目录和检索结果中以诊断报告，其他结果可继续返回。单知识包的目录级错误会终止操作；概览探测或多知识包检索的目录列举失败可以返回部分结果，文件读取与缓存错误仍可能终止整个请求。

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
| `diagnostics` | 诊断 code、message 和可选 path |
| `next` | 建议的后续操作、目标和可选 cursor |

读取视图为 `observed`，上游修订为 null。`metadata` 覆盖状态必须结合 `coverage.scope` 解读：单文件、单目录与全知识包扫描不是同一范围。`body`、`graph` 是保守的覆盖声明，不能解释成离线正文或关系图已经建立。

`page.total` 可以为 null；`has_more=false` 只说明该操作没有续页，不证明全库扫描完整。卡片返回也不意味着 Agent 已读取正文。

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
| 7 | 部分结果，或文件、目录、输出等资源超限 |

完整合规检查发现错误时为 `status: error`、退出码 1；校验扫描未完成时为 `partial`、退出码 7，结果中仍保留已发现的错误。文件过大和输出超限也使用退出码 7，但响应为 `error`。非法 cursor 使用退出码 2，修订变化使用退出码 6。

Agent 应读取稳定的诊断 code，避免匹配错误文本；只引用实际读取的文件、字节范围和内容哈希，并保留影响答案的过期、审核和覆盖限定。消费流程见 [消费 Skill](../skills/okf-consume/SKILL.md)。
