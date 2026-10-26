# CLI 与 Agent 参考

本页描述可执行命令和响应语义。二进制名称为 `okf`，发行包名称为 `okf-cli`。安装和入门见 [README](../README.md)，后续功能见 [实施计划](roadmap.md)。

## 命令

| 命令 | 作用 |
| --- | --- |
| `bundle add NAME --path PATH [--kind fs]` | 检查目录并保存知识包绑定 |
| `bundle list` / `bundle show NAME` | 查看知识包绑定配置 |
| `bundle remove NAME` | 解除绑定，保留知识包文件 |
| `bundle add NAME --kind git --path REPO --ref REF --bundle-path PATH` | 保存固定 commit 的 Git 知识包 |
| `bundle add NAME --kind git --url HTTPS_URL [--ref REF]` | 获取公开 HTTPS 仓库；不能同时传入 `--path` |
| `bundle refresh NAME` | 解析 Git ref 后原子替换提交，失败保留旧绑定 |
| `bundle cache NAME [--compact]` | 查看托管远端对象占用，或重打包压缩；保留历史对象与引用 |
| `context [--bundle NAME]` | 返回知识包概览与目录入口 |
| `context --query QUERY [--bundle NAME]` | 返回元数据候选与原文读取入口 |
| `browse REF` | 展开一层目录 |
| `read REF [--raw]` | 返回原始 UTF-8 文件的一个分页 |
| `search QUERY [--bundle NAME]` | 检索 ID、标题、摘要、标签和类型 |
| `status [NAME]` | 查看已观测元数据的缓存统计 |
| `validate NAME` | 检查知识包中的概念与保留文件 |
| `sync NAME` | 完整扫描后事务发布正文索引 |
| `search QUERY --bundle NAME --full-text` | 检索同步索引的元数据与正文，返回原文片段 |
| `context --query QUERY --bundle NAME --content` | 从同步索引选择较大的原文上下文窗口 |
| `read REF --indexed [--generation GENERATION]` | 离线读取索引原文，可要求与检索结果版本一致 |
| `neighbors REF [--direction incoming\|outgoing\|both]` | 从同步索引查询有证据的直接关系，默认 `both` |
| `index status NAME` / `index clear NAME` | 查看或清理正文索引 |
| `index verify NAME` | 只读检查正文索引的路径、哈希、版本与快照清单 |
| `cache reset` | 归档观测数据库及 sidecar 后重建，活动读取者存在时拒绝 |
| `doctor` | 查看版本、运行目录、项目目录、知识包数量和缓存可用性 |
| `build-info` | 读取内嵌版本、Rust target、构建提交和未提交修改标记；不访问配置或缓存 |
| `skill install PATH` | 将内置消费 Skill 写入指定目录 |
| `help [COMMAND]` / `--help` | 显示文本帮助 |

概览与元数据检索的 `--bundle` 可重复使用，未指定时选择已绑定的全部知识包。全文检索和正文上下文要求至少一个 `--bundle`，重复别名只查询一次。同名项目与用户知识包绑定必须通过 `--scope` 区分。`context` 和 `search` 没有知识包时返回 `NO_BUNDLES`，不会自动扫描当前目录寻找知识包。

别名长度为 1–64 个 ASCII 字符，只能包含字母、数字、`-` 和 `_`。存储类型支持 `fs` 与 `git`，其他类型返回 `UNSUPPORTED_CAPABILITY`。

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

下表列出各命令支持的通用预算与分页选项，不适用的选项返回参数错误。单元格给出默认值，`—` 表示不接受该选项。

| 命令 | `--limit` | `--budget-bytes` | `--max-scan` | `--cursor` | `--timeout-ms` |
| --- | --- | --- | --- | --- | --- |
| `context` 概览 | 20 | 24576 | — | — | 30000 |
| `context --query` | 20 | 24576 | 100 | — | 30000 |
| `browse` | 20 | 16384 | 100 | 可选 | 30000 |
| `read` | — | 65536 | — | 可选 | 30000 |
| `search` | 20 | 16384 | 100 | — | 30000 |
| `neighbors` | 20 | 16384 | 100 | — | 30000 |
| `validate` | — | — | 100 | — | 30000 |
| `sync` | — | — | 100 | — | 30000 |
| `index verify` | — | — | 100 | — | 30000 |

`--full-text`、`--content` 和 `--indexed` 沿用对应命令的预算与分页参数。全文检索的 `--max-scan` 限制评估的概念数量，`indexed_documents_scanned` 返回实际数量。单包索引总正文上限 64 MiB、文档数量上限 100000；索引单文件读取上限 2 MiB。`validate --max-resources N` 控制资源声明数量，默认 4096，允许 1–10000；声明字符串另有 2 MiB 总上限。

取值范围：`--limit` 为 1–100，`--budget-bytes` 为 1–65536，`--max-scan` 为 1–100000，`--timeout-ms` 为 1–3600000 毫秒。

原文预算计算返回文本的 UTF-8 字节数，条目预算计算序列化条目字节数；完整响应另外受 `--max-output-bytes` 限制。原文按 Unicode 字符边界分页，放不下下一个字符时返回 `BUDGET_TOO_SMALL`。浏览、元数据搜索及候选上下文跳过单张超过整个预算的卡片，附带路径和 `CARD_TOO_LARGE`，继续返回其他条目；浏览的剩余预算放不下下一张正常卡片时分页。概览预算不足时报告 `BUDGET_EXCEEDED`。

扫描预算在文件系统适配器内扣减：打开一个目录计一次，每次推进目录迭代器计一次，包括非 Markdown 文件与确认目录结束的那一步。文件系统的所有目录共用本次请求的预算，不会先列举整个目录再截断。Git 列举先读取有大小上限的 tree 输出，再按条目扣减扫描预算。`budget.scan_limit` 和 `budget.scanned_entries` 返回上限与实际计数。单文件读取上限为 2 MiB。

目录清单按路径排序，分页 cursor 校验完整清单的修订。列举预算不足时，`browse` 返回已观测条目的部分结果、`SCAN_LIMIT` 和退出码 7，不生成 cursor；应提高 `--max-scan` 后重新浏览。带 cursor 的请求无法在预算内重建完整清单时返回 `SCAN_LIMIT` 错误，不使用不完整清单验证修订。

有后续分页时，`page.has_more` 为 true，`page.next_cursor` 和 `next` 给出继续入口。将 cursor 原样传给相同知识包、目标和操作；遇到 `REVISION_CHANGED` 时重新开始，不拼接不同修订。正常目录和原文分页返回退出码 0；搜索没有续页，达到结果或扫描上限时应缩小范围或提高预算。

## 关系导航与扩展检查

`neighbors` 需要先执行 `sync`，离线工作并返回 generation。它只提取 Markdown 概念链接与 `sources[].resource` 中的包内 `.md` 引用；正文相对路径以文档目录为基准，来源资源相对路径按包根解析。外部 URI、目录、附件和图片不构成概念边；不根据文字相似性推断关系。

每个结果包含 `from`、`to`、方向、目标是否存在，以及证据文件哈希和正文 `location` 或元数据 `field`。缺失目标仍保留，循环关系只报告直接边。`--limit`、`--budget-bytes`、`--max-scan`、`--timeout-ms` 默认值与搜索一致；扫描预算计索引文档数，单文档正文链接与来源各最多 256 条。`relationship_check.complete` 描述关系扫描，结果选择不足另报 `RESULT_LIMIT`；扫描或引用截断时不能把没有入链解释成全包没有引用。

`validate --attachments` 用元信息查询检查非 Markdown 正文链接的存在性与文件类型，不读取附件内容，不检查图片。结果独立返回 `attachment_check`；与正文链接共用数量和字符串预算。

`validate --anchor-style headings-v1` 显式启用标题锚点规则：提取 Markdown 标题文字与行内代码，转小写，空白替换为 `-`，保留字母、数字、`_`、`-`，重复 ID 顺序追加 `-1` 等后缀。解析 URL 百分号编码后匹配。该规则是 CLI 扩展，不代表所有渲染器；默认不检查锚点。存在自定义 HTML、无法解码的片段或不完整标题清单时，将无法确定的目标计入 `anchor_check.skipped`，不产生缺失锚点误报。每文件最多 4096 个标题，包内标题清单总估算占用最多 2 MiB；超限或扫描中断分别报告覆盖。

## Git 远端与缓存

`bundle add`、`bundle refresh`、`bundle cache` 支持 `--timeout-ms`，默认 30000。公开 HTTPS 获取支持 `--max-git-bytes`，默认 268435456（256 MiB）。它是托管仓库对象目录的总占用预算，获取前、获取期间约每 50 ms 及获取后检查；可能存在检查间隔内的超量写入，不是精确网络字节上限。超限不发布新绑定，已获取的对象可能保留，后续可调整预算。

同一托管仓库的获取、刷新与压缩通过系统锁互斥。`bundle cache NAME --compact` 仅作用于运行目录内的托管远端仓库，保留历史引用及不可达对象，消除冗余打包占用；不对用户本地仓库执行清理，也不实施历史版本过期删除。

远端失败使用 `GIT_AUTH_FAILED`、`GIT_TLS_FAILED`、`GIT_REMOTE_NOT_FOUND` 或 `GIT_TRANSPORT_FAILED`，对象占用超限使用 `GIT_STORAGE_LIMIT`（7），写锁竞争使用 `GIT_CACHE_LOCKED`（5）。原始 Git stderr 不进入响应。普通对象缺失仍使用 `GIT_OBJECT_UNAVAILABLE`。网络失败与预算不足均保留旧绑定。

## 截止时间与取消

消费命令支持 Ctrl+C 取消，诊断为 `CANCELLED`；超过截止时间时为 `DEADLINE_EXCEEDED`，均使用退出码 7。检查点覆盖目录遍历、逐文件处理和分块读取。概览、检索和校验保留已完成结果；浏览在处理条目时中断也保留已有结果，但无法完成初始目录列举时返回错误。原文读取中断返回错误。

截止时间从请求执行开始计时，在检查点协作生效；操作系统中已阻塞的单次 I/O、数据库操作与序列化不能被强制中断。这个选项不能作为进程运行时间的硬上限。

## 搜索与生命周期

`search` 与 `context --query` 支持 `--type`、`--tag`、`--status` 精确匹配；同一选项重复表示 OR，不同字段之间为 AND，大小写敏感。筛选发生在结果截断之前，实时与索引模式采用相同规则。不带查询的概览拒绝这些筛选选项。

`--trust` 接受 `unverified`、`machine-confirmed`、`human-reviewed`；`--expiry` 接受 `current`、`expired`、`unspecified`、`unknown`，同样支持重复取值。未声明有效期与日期无效分别为 `unspecified` 和 `unknown`。默认不按审核层级或有效期排除结果。

`--as-of TIMESTAMP` 只用于搜索和查询上下文，接受带 UTC 偏移的 RFC 3339 时间，默认使用请求时间。结果的 `data.evaluated_at` 记录归一化后的 UTC 时间；到达 `stale_after` 时即为过期。索引查询从已保存的原文按本次评估时间派生有效期，不沿用同步时的结论。它不改变观测时间、索引 generation 或缓存记录中的真实观测时间；未知元数据值的诊断仍保留。`--as-of` 不证明当时的源文件内容。

查询按空白拆词，忽略大小写，并要求所有词都在卡片的检索字段中出现。支持中文子串，结果按知识包引用排序。`context --query` 复用元数据检索，返回 `disclosure: metadata_candidates` 与 `read` 入口。

全文模式要求每个词出现在卡片或正文中，元数据命中每词计 5 分、仅正文命中每词计 1 分，同分按引用排序。它是确定性子串检索，未使用向量或分词模型。结果的 `snippet` 含原始字节范围与文本，普通片段最多 512 字节，正文上下文最多 4096 字节，均受整体条目预算约束。标题边界来自 Markdown 解析器，代码块中的 `#` 不作为标题。上下文从包含命中的最近章节开始，在下一个同级或上级标题前结束；标题距命中过远时从命中字节位置开始。每份文档只返回一个窗口，避免同一文档的重叠片段；不合并不同文件的来源。`coverage_gaps` 表达未选命中和未检查实时文件的事实。

`sync` 只在完整、无结构错误的扫描后发布 generation；取消、预算不足或文件错误均保留旧索引。并发发布基于旧 generation 比较，冲突返回 `INDEX_CHANGED`（6）。`read --indexed` 的 cursor 绑定 generation；`--generation` 可在首个分页就拒绝过时的检索引用。索引视图的 `freshness` 为 `cached`，`checked_at` 为同步时间；本地文件索引仍是观测视图。`matches_binding_revision` 对 Git 表示索引提交是否等于当前绑定提交，对文件系统不证明源文件未变。

正文片段使用的标题清单每文件最多 4096 项；超过时返回 `SECTION_LIMIT` 和部分结果，片段仍保留精确字节定位，但章节边界可能不完整。

卡片中的 `trust`、`status` 和 `expiry` 从元数据派生，默认值与异常值的处理见 [派生与覆盖](okf-validation.md#派生与覆盖)。这些字段表达源文件声明，不提供身份验证或内容真实性保证。

多包索引查询的 `--limit`、`--budget-bytes`、`--max-scan` 和 `--timeout-ms` 属于整个请求。按包名排序，将扫描配额轮流分配给仍有概念待评估的包；小包用不完的份额在读取前分给其他包。配额小于包数时，后面的包可能分不到扫描份额，并报告 `SCAN_LIMIT`。每包按路径顺序加载配额内的概念，再按统一得分和完整引用排序、应用全局结果及字节限制。配额分配不保证每包都有命中；故障或取消后的剩余份额不重新分配。

单包结果保留 `data.generation`；多包结果使用 `data.generations` 映射别名到版本，条目与 `next` 各自携带 generation。每项还包含 `bundle_id`。重复正文和冲突证据保持独立引用。各包是独立快照，不构成跨包同一时刻的快照。

多包查询中的缺失、损坏或读取期间变化的索引产生带包路径的诊断，保留其他包结果并返回退出码 7。无法读取的包列入 `coverage_gaps.unavailable_bundles`；任何包未完成评估时 `page.total` 为 null。有效包的观测包含 `scan_limit` 和 `indexed_documents_scanned`。单包请求仍直接返回对应错误；未知或有歧义的绑定在查询前拒绝。

`index status` 只查询索引元信息和统计，不验证正文完整性。`read --indexed` 在同一事务中读取版本与目标文档，仅验证该文档哈希；全文查询验证扫描配额内的正文。哈希不匹配返回 `INDEX_CORRUPT`，可重新执行 `sync NAME` 从源文件恢复正文；源内容未变时 generation 保持不变。

## 索引完整性检查

`index verify NAME` 使用只读数据库连接，在同一事务快照内检查所有类型的索引文档，包括保留文件。按路径检查合法性、正文 SHA-256 和文档 generation；完整扫描后，再从保存的路径、哈希和上游修订重算快照 generation。它不验证源文件新鲜度，也不替代 `validate` 的格式检查。

`data.documents_total` 为当前快照的文档数，`documents_checked` 为本次已检查数量，`complete` 表示检查是否完成。`valid` 只有在完整检查且无问题时为 true；已发现问题时为 false；未完成且尚无问题时为 null。`failures` 统计发现的问题数，一份文档可有多个问题；完整性问题最多保留 100 项诊断，超出时 `findings_truncated` 为 true，问题计数继续累加。扫描上限、截止时间或取消诊断另行保留。

完整且无问题返回 0，完整且发现损坏返回 5；扫描、64 MiB 检查预算或截止时间耗尽、取消时返回 7，保留已完成的检查结果，即使已发现损坏。单份正文超过 2 MiB 时报告 `INDEX_DOCUMENT_LIMIT`。其他完整性诊断包括 `INDEX_PATH_INVALID`、`INDEX_CORRUPT`、`INDEX_GENERATION_MISMATCH` 和 `INDEX_SNAPSHOT_MISMATCH`。缺失索引返回 3，且不创建正文数据库；不支持的数据库版本返回 `INDEX_VERSION`（2），不执行迁移。检查不会修复内容；显式 `sync` 在事务内重建该包的完整文档清单，包括移除多余记录。

`budget.scan_limit` 和 `indexed_documents_scanned` 记录文档扫描上限及实际数量，`index_bytes_limit` 和 `index_bytes_checked` 记录正文检查的累计字节上限及计数。`data.live_files_checked` 固定为 false。

## 故障与覆盖

浏览或检索遇到坏概念、文件读取失败时，返回带路径的诊断和部分结果，继续处理可用条目。多知识包概览和检索隔离知识包故障，保留其他包的结果；单知识包初始目录不可读时返回错误。未开始读取、探测失败或未能展开目录的包使用 `freshness: unknown`、`checked_at: null` 与未知覆盖状态。

实时读取命令会尝试打开 SQLite 观测缓存。消费命令遇到缓存故障时保留可读内容：写入失败使用 `CACHE_WRITE_FAILED`，打开失败且本次操作没有尝试写入卡片时使用 `CACHE_UNAVAILABLE`。索引命令使用独立数据库，不依赖观测缓存。`cache reset` 在不打开损坏数据库的情况下归档重建。配置写入使用系统锁，进程退出后自动释放；锁文件应保留。

缓存问题与知识文件覆盖分别表达，完整校验发现的文档错误优先于缓存诊断。`SCAN_LIMIT` 仅用于扫描预算耗尽，文件故障或取消不会被标为预算不足。

`validate` 返回 `data.link_check`，其中 `scope` 为 `bundle_markdown_targets`，`complete` 为布尔值。它说明包内 Markdown 链接与目录目标检查是否完成；扫描未完成时跳过目标存在性判断，读取失败、链接预算耗尽、正文收集或警告截断，以及取消都会使其为 false。它不表示目标全部有效或关系图已建立，检查范围见 [正文链接与来源脚注](okf-validation.md#正文链接与来源脚注)。

`budget.link_limit` 与 `link_bytes_limit` 给出待检查链接的数量、字符串字节上限；`links_checked` 是实际检查的链接数，`link_bytes_used` 是保留链接占用的字符串字节数。`LINK_LIMIT` 返回退出码 7，链接阶段取消或预算耗尽均保留已完成的元数据覆盖。具体上限见 [诊断上限](okf-validation.md#诊断上限)。

计算资源检查独立返回 `data.resource_check`，范围为 `bundle_computation_resources`。缺失文件或目录类型不符产生字段级警告；权限、越界、查询失败使用操作诊断和部分结果。查询预算耗尽为 `RESOURCE_LIMIT`（7），不降低已完成的元数据扫描覆盖。外部 URI 仅计入 `external_skipped`，不访问。`resources_checked`、`resource_limit`、`resource_bytes_used`、`resource_bytes_limit` 给出预算。

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

文件系统读取视图为 `observed`，上游修订为 null；Git 视图为 `snapshot`，`upstream_revision` 是完整 commit ID。Git 原文、卡片与 cursor 均携带或绑定提交。`metadata` 覆盖状态必须结合 `coverage.scope` 解读：单文件、单目录与全知识包扫描不是同一范围。实时命令的 `body`、`graph` 是保守覆盖声明；索引的完整正文覆盖仅描述已发布的索引版本。

`page.total` 可以为 null；`has_more=false` 只说明该操作没有续页，不证明全库扫描完整。卡片返回也不意味着 Agent 已读取正文。

`diagnostics[].severity` 为 `error` 或 `warning`。元数据及正文警告的 `path` 为完整文件引用，如 `@handbook/services/auth.md`；元数据的 `field` 为 frontmatter 的 JSON Pointer，如 `/verified/0/at`。正文、操作或文档级诊断的 `field` 为 null，包括截断提示。默认情况下，警告不改变执行状态或元数据覆盖；显式启用质量门槛时按下文规则判定退出码。诊断条数规则见 [诊断上限](okf-validation.md#诊断上限)。

正文诊断通过可选的 `location: {byte_start, byte_end}` 标识完整原文件中的 UTF-8 字节半开区间，包含 BOM、frontmatter 与换行所占字节；它不相对于当前读取分页。没有具体正文位置的诊断省略此字段。

缺少内联计算块时，`location` 指向已有的计算章节标题；计算章节也缺失时，使用正文起点的零长度区间表示应补充内容的位置。计算合同的字段与正文规则见 [计算合同与正文](okf-validation.md#计算合同与正文)。

## 退出码

| 退出码 | 含义 |
| --- | --- |
| 0 | 成功，包括正常分页、完整范围内零命中或过期内容 |
| 1 | 文档不合规或合规检查发现错误 |
| 2 | 参数、配置、知识包歧义或不支持的能力 |
| 3 | 知识包、目录或文件不存在 |
| 4 | 访问被拒绝、路径越界或目录链接被拒绝 |
| 5 | I/O、缓存或配置锁错误 |
| 6 | 读取期间内容变化、分页修订变化或索引发布版本冲突 |
| 7 | 部分结果、资源超限、截止时间或取消 |

完整合规检查发现错误时为 `status: error`、退出码 1，缓存不可用诊断不会覆盖这个结果；校验扫描未完成时为 `partial`、退出码 7，结果中仍保留已发现的错误。文件过大和输出超限也使用退出码 7，但响应为 `error`。非法 cursor 使用退出码 2，修订变化使用退出码 6。

`validate` 返回 `data.failures` 与 `data.warnings`，分别统计失败文件与返回的元数据、正文及资源警告。只有警告且检查完成时仍为 `ok`、退出码 0。

显式指定 `validate --max-warnings N`（允许 0）启用质量门槛。`data.quality_gate` 返回 `max_warnings`、`warnings_observed`、`warnings_total`、`complete` 和 `passed`。观测数不包含诊断截断提示；完整时总数等于观测数，不完整时总数为 null。已观测数量超过门槛即确定 `passed: false`；未超过且统计不完整时为 null，只有完整且未超过时才为 true。

扫描、链接或资源预算耗尽，读取失败，诊断截断，以及已启用检查中的未知目标都会阻止门槛通过。完整检查超限返回 `WARNING_LIMIT_EXCEEDED`、退出码 1；不完整检查优先保持退出码 7，只有警告截断等导致计数不完整而此前尚未标记部分结果时增加 `QUALITY_GATE_INCOMPLETE`。已确定的结构错误保留退出码 1；缓存等操作错误不会被质量门槛覆盖。即使 `passed: true`，仍需检查响应执行状态。默认未指定门槛时行为不变。

Agent 应读取稳定的诊断 code，避免匹配错误文本；只引用实际读取的文件、字节范围和内容哈希，并保留影响答案的过期、审核和覆盖限定。消费流程见 [消费 Skill](../skills/okf-consume/SKILL.md)。
