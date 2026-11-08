# CLI 使用与校验参考

本页供 CLI 使用者和 Agent 查询命令、响应语义及 OKF 格式校验规则。二进制名称为 `okf`，发行包名称为 `okf-cli`。安装和入门见 [README](../README.md)，后续功能见 [实施计划](architecture.md#实施计划)。

常用定位：[命令](#命令) · [预算与分页](#预算与分页) · [Git](#git-远端与缓存) · [搜索](#搜索与生命周期) · [JSON 响应](#json-响应) · [退出码](#退出码) · [OKF 检查矩阵](#okf-检查矩阵)。

## 命令

| 命令 | 作用 |
| --- | --- |
| `bundle add NAME --path PATH [--kind fs]` | 检查目录并保存知识包绑定 |
| `bundle list` / `bundle show NAME` | 查看知识包绑定配置 |
| `bundle remove NAME` | 解除绑定，保留知识包文件 |
| `bundle add NAME --kind git --path REPO --ref REF --bundle-path PATH` | 保存固定 commit 的 Git 知识包 |
| `bundle add NAME --kind git --url URL [--ref REF] [--credential env:VAR\|file:PATH] [--max-fetch-bytes N]` | 获取公开或私有 HTTPS/SSH 仓库；不能同时传入 `--path` |
| `bundle refresh NAME [--max-fetch-bytes N]` | 解析 Git ref 后原子替换提交，失败保留旧绑定 |
| `bundle cache NAME [--compact] [--gc]` | 查看托管远端对象占用、重打包压缩或显式回收历史对象 |
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

`skill install PATH` 创建 `PATH/SKILL.md` 和 `PATH/references/cli-workflows.md`，写入二进制内嵌内容；`PATH` 不决定 Skill 名称。当前源码内嵌 `name: okf-cli`，示例为 `okf skill install .agents/skills/okf-cli`。安装不打开绑定配置或数据库，不迁移旧目录；任一目标文件已存在时返回退出码 5 并保留原内容。已发布版本与当前源码的名称差异见 [Skill 更名与升级](releases.md#skill-更名与升级)。

概览与元数据检索的 `--bundle` 可重复使用，未指定时选择已绑定的全部知识包。全文检索和正文上下文要求至少一个 `--bundle`，重复别名只查询一次。同名项目与用户知识包绑定必须通过 `--scope` 区分。`context` 和 `search` 没有知识包时返回 `NO_BUNDLES`，不会自动扫描当前目录寻找知识包。

别名长度为 1–64 个 ASCII 字符，只能包含字母、数字、`-` 和 `_`。存储类型支持 `fs` 与 `git`，其他类型返回 `UNSUPPORTED_CAPABILITY`。

知识包、概念、来源与绑定别名的定义见 [领域术语](architecture.md#领域术语)。配置中的 `bundles` 保存绑定，响应中的同名字段保存读取观测；概念的 `sources` 专指依据材料。

### 绑定与管理操作边界

- `bundle add` 必须指定 `--path` 或 `--url`，两者互斥；`--kind` 默认 `fs`。`--url`、`--ref`、`--bundle-path`、两个 Git 字节预算及 `--credential` 只适用于 `git`；凭据引用还要求 `--url`。同一作用域重复别名返回 `BUNDLE_EXISTS`（2），不覆盖原配置。
- Git 的 `--ref` 默认 `HEAD`，`--bundle-path` 默认仓库根目录；本地 `--path` 固定已有仓库中的提交，不执行 fetch。对本地绑定执行 `bundle refresh` 仅重新解析该仓库中的 ref；远端绑定刷新才获取远端对象。两个获取字节预算对本地仓库解析没有计量作用。
- `bundle list` 按别名、作用域排序，列出配置而不逐包检查内容；`bundle show` 返回所选绑定，包含 Git 固定提交及凭据引用，不包含 Token。`bundle remove` 只删除绑定，不清理观测缓存、正文索引或远端对象。
- `status [NAME]` 查询历史卡片数量和最近观测时间，不重新扫描文件，也不判断正文索引新鲜度。`doctor` 报告版本、运行目录、项目目录、绑定数和已成功打开的 SQLite 状态，不逐包校验内容或测试网络。
- `cache reset` 只归档并重建 `index.sqlite` 观测数据库及 sidecar；`index clear NAME` 只删除该绑定的正文快照与候选派生数据，保留其他知识包。两者都保留源知识包和绑定，`cache reset` 不清除 `content.sqlite` 或 Git 对象。

`--version` 显示包版本；`build-info` 还返回 `target`、`source_commit`、`source_dirty`，构建时无法确定来源的字段为 null。同一版本号可能对应不同构建，不能只据版本号判断未发布功能。

### 参数组合

`read --raw` 与 `--indexed` 互斥；`--generation` 必须与 `--indexed` 同用。`search --full-text` 必须指定 `--bundle`；`context --content` 必须同时指定 `--query` 和 `--bundle`。概览不接受 `--max-scan` 或搜索筛选条件；`--candidate-budget` 只用于两种索引查询。`--cursor` 仅用于 `browse` 和 `read`，关系查询和搜索没有续页。

## 全局选项

| 选项 | 默认值 | 含义 |
| --- | --- | --- |
| `--format FORMAT` | `text` | `text` 优先输出缩进 JSON（超出输出预算时回退紧凑 JSON），`json` 为紧凑 JSON |
| `--scope SCOPE` | 读取两个作用域；新增绑定写入项目作用域 | 使用 `project` 或 `user` 限定配置知识包 |
| `--home PATH` | `OKF_HOME`，否则系统本地数据目录下的 `okf-cli` | 指定绝对运行目录 |
| `--max-output-bytes N` | 131072 | 整个序列化响应的字节上限；允许 1024–1048576 |

业务响应的 stdout 是单个 JSON 对象，不混入日志。帮助和版本输出是文本，包括使用 `--format json` 时。参数错误在显式请求 JSON 时使用相同响应 envelope。

`--home` 优先于 `OKF_HOME`，两者都要求绝对路径。项目根目录是从当前目录向上找到的最近一个含 `.okf-cli/config.toml` 或 `.git` 的目录；找不到时使用当前目录。用户配置位于运行目录的 `config.toml`，项目配置位于项目根目录的 `.okf-cli/config.toml`。项目内知识包保存相对路径；读取配置时，项目作用域的相对根路径按项目根目录解析，用户作用域的相对根路径按运行目录解析。

全局选项可用于各子命令，但 `build-info`、`skill install`、帮助与版本不打开运行配置或数据库；`--home`、`--scope` 对这些操作不改变读取目标。其余管理命令也可能因配置或观测缓存无法打开而失败，`doctor` 不是绕过损坏状态的恢复入口。

## 引用与原文

`browse @handbook` 展开知识包根目录，`browse @handbook/services` 展开子目录。`read @handbook/services/auth` 读取 `services/auth.md`，也可以传入完整文件名。以 `.md` 结尾的读取引用按完整文件名处理。

`browse` 的目录索引使用 `index.origin` 区分 `authored` 与 `synthesized`；目录未完整列举且尚未发现索引时为 `unknown`。作者索引通过 `index.ref` 单独读取，不嵌入目录结果。`index.md` 与 `log.md` 可直接传给 `read`。

普通概念的 frontmatter 必须包含非空字符串 `type`。`read --raw` 可查看不合规 Markdown，并附解析诊断；无效 UTF-8 和大小限制仍会阻止读取。`validate` 还检查根索引 frontmatter 的允许字段、子目录索引限制及日志日期标题。

结构、元数据与正文诊断规则见 [OKF 检查矩阵](#okf-检查矩阵)。质量警告保留概念与原文；缺少可选字段、未知类型或扩展字段不触发警告。

`read` 在 `data.document` 中返回 `bundle_id`、`path`、`concept_id`、`content_sha256`、`byte_start`、`byte_end`、`total_bytes` 和 `text`。字节范围为完整原始文件中的半开区间，包括 BOM、frontmatter 和换行。拼接同一修订的分页 `text` 可以还原文件。

## 预算与分页

下表列出各命令支持的通用预算与分页选项，不适用的选项返回参数错误。单元格给出默认值，`—` 表示不接受该选项。`bundle add`、`bundle refresh`、`bundle cache` 另接受 `--timeout-ms`（默认 30000）；其他未列出的管理命令不接受这些通用预算。

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

`--full-text`、`--content` 和 `--indexed` 沿用对应命令的预算与分页参数。全文检索的 `--max-scan` 限制评估的概念数量，`indexed_documents_scanned` 返回实际数量。单包索引总正文上限 64 MiB、文档数量上限 100000；索引单文件读取上限 2 MiB。`validate --max-resources N` 控制资源声明数量，默认 4096，允许 1–100000；声明字符串另有 2 MiB 总上限。

取值范围：`--limit` 为 1–100，`--budget-bytes` 为 1–65536，`--max-scan` 为 1–100000，`--timeout-ms` 为 1–3600000 毫秒。

原文预算计算返回文本的 UTF-8 字节数，条目预算计算序列化条目字节数；完整响应另外受 `--max-output-bytes` 限制。原文按 Unicode 字符边界分页，放不下下一个字符时返回 `BUDGET_TOO_SMALL`。浏览、元数据搜索及候选上下文跳过单张超过整个预算的卡片，附带路径和 `CARD_TOO_LARGE`，继续返回其他条目；浏览的剩余预算放不下下一张正常卡片时分页。概览预算不足时报告 `BUDGET_EXCEEDED`；概览的 `--limit` 计知识包数，超限报告 `BUNDLE_LIMIT`，不生成续页，需显式缩小 `--bundle` 范围。

扫描预算在文件系统适配器内扣减：打开一个目录计一次，每次推进目录迭代器计一次，包括非 Markdown 文件与确认目录结束的那一步。文件系统的所有目录共用本次请求的预算，不会先列举整个目录再截断。Git 列举先读取有大小上限的 tree 输出，再按条目扣减扫描预算。`budget.scan_limit` 和 `budget.scanned_entries` 返回上限与实际计数。单文件读取上限为 2 MiB。

目录清单按路径排序，分页 cursor 校验完整清单的修订。列举预算不足时，`browse` 返回已观测条目的部分结果、`SCAN_LIMIT` 和退出码 7，不生成 cursor；应提高 `--max-scan` 后重新浏览。带 cursor 的请求无法在预算内重建完整清单时返回 `SCAN_LIMIT` 错误，不使用不完整清单验证修订。

有后续分页时，`page.has_more` 为 true，`page.next_cursor` 和 `next` 给出继续入口。将 cursor 原样传给相同知识包、目标和操作；遇到 `REVISION_CHANGED` 时重新开始，不拼接不同修订。正常目录和原文分页返回退出码 0；搜索没有续页，达到结果或扫描上限时应缩小范围或提高预算。

## 关系导航与扩展检查

`neighbors` 需要先执行 `sync`，离线工作并返回 generation。它只提取 Markdown 概念链接与 `sources[].resource` 中的包内 `.md` 引用；正文相对路径以文档目录为基准，来源资源相对路径按包根解析。外部 URI、目录、附件和图片不构成概念边；不根据文字相似性推断关系。

`--direction outgoing` 只解析目标文档，`incoming` 和 `both` 按索引文档顺序扫描全包；加载阶段仍读取有界完整快照，因此 `--max-scan` 不限制加载快照的总内存。相同两份文档间的不同引用保留为独立证据，不进行传递关系推导或关系去重。

每个结果包含 `from`、`to`、方向、目标是否存在，以及证据文件哈希和正文 `location` 或元数据 `field`。缺失目标仍保留，循环关系只报告直接边。`--limit`、`--budget-bytes`、`--max-scan`、`--timeout-ms` 默认值与搜索一致；扫描预算计索引文档数，单文档正文链接与来源各最多 256 条。`relationship_check.complete` 描述关系扫描，结果选择不足另报 `RESULT_LIMIT`；扫描或引用截断时不能把没有入链解释成全包没有引用。

`validate --attachments` 用元信息查询检查非 Markdown 正文链接与图片引用的存在性和文件类型，不读取附件内容。结果独立返回 `attachment_check`，其中 `images_included` 恒为 `true`、`images_checked` 统计图片引用数量，`types` 列出检查到的附件目标的小写扩展名（去重、升序，图片与普通附件一并计入）；与正文链接共用数量和字符串预算。

`validate --anchor-style headings-v1` 显式启用标题锚点规则：提取 Markdown 标题文字与行内代码，转小写，空白替换为 `-`，保留字母、数字、`_`、`-`，重复 ID 顺序追加 `-1` 等后缀。解析 URL 百分号编码后匹配。此外，从原始 HTML 中提取 `id` 与 `name` 属性值作为显式锚点一并匹配，属性值中的字符引用（如 `&amp;`、`&#45;`、`&#x2D;`）先解码再匹配。该规则是 CLI 扩展，不代表所有渲染器；默认不检查锚点。存在无法识别的自定义 HTML、无法解码的片段或不完整标题清单时，将无法确定的目标计入 `anchor_check.skipped`，不产生缺失锚点误报。每文件最多 4096 个标题，包内标题清单总估算占用最多 2 MiB；超限或扫描中断分别报告覆盖。

## Git 远端与缓存

Git 适配器需要 Git 可执行文件，可用 `OKF_GIT` 指定路径。需要代理时，通过 `HTTPS_PROXY` / `HTTP_PROXY` 传给 Git；不会自动读取操作系统浏览器代理。读取使用固定 commit 的 tree/blob，不检出工作区。私有仓库与 SSH 等能力是否已发布，见 [发行范围](releases.md#下载与安装)。

`bundle add`、`bundle refresh`、`bundle cache` 支持 `--timeout-ms`，默认 30000。远端 HTTPS/SSH 获取支持 `--max-git-bytes`，默认 268435456（256 MiB）。它是托管仓库对象目录的总占用预算，获取前、获取期间约每 50 ms 及获取后检查；可能存在检查间隔内的超量写入，不是精确网络字节上限。超限不发布新绑定，已获取的对象可能保留，后续可调整预算。

`bundle cache NAME` 不带动作时只报告对象占用；`--gc` 和 `--compact` 同时指定时执行 GC 分支。维护只接受运行目录 `git` 下直接管理的远端仓库，本地 Git 绑定返回 `UNSUPPORTED_CAPABILITY`（2）。

同一托管仓库的获取、刷新与压缩通过系统锁互斥。`bundle cache NAME --compact` 仅作用于运行目录内的托管远端仓库，保留历史引用及不可达对象，消除冗余打包占用；不对用户本地仓库执行清理，也不实施历史版本过期删除。

私有 HTTPS 仓库通过 `--credential` 引用凭据，支持 `env:VAR`（从环境变量读取）与 `file:PATH`（从文件读取；Unix 拒绝任何组或其他用户权限位，0400/0600 均可，Windows 不执行此权限位检查）。新增绑定时，`file:PATH` 的相对路径按调用目录转换为绝对路径并保存，后续刷新不受工作目录变化影响；已有绑定中的相对凭据路径应改为绝对路径。解析后的 token 只注入 Git 子进程环境，绝不写入配置、命令行参数、响应或日志；环境变量缺失、token 为空或含非法控制字符返回 `GIT_CREDENTIAL_MISSING`（5）；凭据文件读取失败按 I/O 错误返回。

SSH 仓库通过 `ssh://` 或 `user@host:path` 形式的 `--url` 获取，鉴权由系统 Git 与用户 `~/.ssh/config`（SSH 密钥、代理）处理；`--credential` 仅适用于 HTTPS。

`bundle add`/`bundle refresh` 在 Git fetch 成功后，若本机已安装 git-lfs，会以 best-effort 方式运行 `git lfs fetch` 下载该提交引用的 LFS 对象到托管仓库的 `lfs/objects` 缓存；失败（未安装 git-lfs、远端无 LFS 端点或网络中断）不影响 Git 绑定发布；存储配额超限、取消和截止时间仍会阻止发布。LFS 传输使用独立的 HTTP batch 协议，`--credential`（Git HTTP 鉴权）不适用于 LFS。

Git LFS 指针在读取时从本地缓存取回真实内容：`read` 遇到以 `version https://git-lfs.github.com/spec/v1` 开头的 LFS 指针时，从托管仓库的 `lfs/objects`（或 `.git/lfs/objects`）缓存读取对象原文，而非返回指针占位文本。对象未下载时返回 `LFS_CONTENT_UNAVAILABLE`（7），不触发网络下载、不返回指针；对象超过读取上限仍返回 `DOCUMENT_TOO_LARGE`。返回内容前核对指针声明的长度与 SHA-256；不匹配或使用不支持校验的 SHA-1 指针时返回 `LFS_INTEGRITY_ERROR`（5）。

子模块仅在 HTTPS 主机和端口与主仓库完全相同时继承 `--credential`；跨主机、不同端口或不同协议不转发 Token。SCP 格式地址（如 `git@host:org/repo.git`）保持原样，仅 `./`、`../` 开头的 URL 按主仓库地址解析。

fetch 后根据主仓库 `.gitmodules` 和 gitlink 获取直接子模块对象，不递归下载嵌套子模块。读取时将本地已有的 gitlink 提交解析为目录。子模块获取为 best-effort，普通传输失败不使主仓库绑定失效；存储配额超限、取消和截止时间会阻止新绑定发布并保留旧绑定。读取缺失对象时返回 `GIT_OBJECT_UNAVAILABLE`。传输仅允许 HTTPS/SSH，不跟随本地路径或 `file://` URL。

`--max-fetch-bytes` 限制主仓库单次 fetch 新增对象的对象字节和（fetch 前后 `cat-file --batch-all-objects` 差集），默认不设置；检查在 LFS 与子模块获取之前完成，不涵盖它们的新增对象，与采样的 `--max-git-bytes` 存储配额相互独立；超限返回 `GIT_FETCH_LIMIT`（7）并保留旧绑定。它计量对象字节，不含 TLS 与传输开销。

`bundle cache NAME --gc` 显式回收历史与不可达对象：先递归扫描当前项目与作用域选择下、指向同一仓库的可见绑定提交及其 gitlink 子模块（包括嵌套子模块），为这些提交建立临时 pin ref，删除不再指向存活提交的临时 fetch ref，再执行 `git gc --prune=now`。扫描最多检查 100000 个树条目；对象缺失、扫描超限或 pin 创建失败时不执行回收。默认不自动过期，清理不影响仍使用的提交及已下载的子模块内容。

远端失败使用 `GIT_AUTH_FAILED`、`GIT_TLS_FAILED`、`GIT_REMOTE_NOT_FOUND` 或 `GIT_TRANSPORT_FAILED`，对象占用超限使用 `GIT_STORAGE_LIMIT`（7），写锁竞争使用 `GIT_CACHE_LOCKED`（5）。原始 Git stderr 不进入响应。普通对象缺失仍使用 `GIT_OBJECT_UNAVAILABLE`。网络失败与预算不足均保留旧绑定。

## 截止时间与取消

消费命令支持 Ctrl+C 取消，诊断为 `CANCELLED`；超过截止时间时为 `DEADLINE_EXCEEDED`，均使用退出码 7。检查点覆盖目录遍历、逐文件处理和分块读取。概览、检索和校验保留已完成结果；浏览在处理条目时中断也保留已有结果，但无法完成初始目录列举时返回错误。原文读取中断返回错误。

截止时间从请求执行开始计时，在检查点协作生效；操作系统中已阻塞的单次 I/O、数据库操作与序列化不能被强制中断。这个选项不能作为进程运行时间的硬上限。

## 搜索与生命周期

`search` 和 `context --query` 支持单个 `--dir PATH`，表示各所选知识包内相同的相对目录及其全部子目录。允许一个末尾 `/`，拒绝空值、绝对路径、反斜杠及 `.`、`..` 路径段；查询包根时省略该选项。路径区分大小写，`services` 不包含 `services-old`，`%` 和 `_` 按字面匹配。

实时模式从该目录开始列举，扫描预算仍按目录遍历计数；目录不存在时沿用文件系统读取错误。索引模式按保存路径的范围限定统计和候选，然后分配共享扫描预算，不加载其他目录正文；不存在的范围返回完整的零命中。`data.directory` 和每包 `coverage.scope` 标明目录，`page.total` 与完整性只描述该范围。元数据与正文匹配规则保持一致，未指定目录时行为不变。

`search` 与 `context --query` 支持 `--type`、`--tag`、`--status` 精确匹配；同一选项重复表示 OR，不同字段之间为 AND，大小写敏感。筛选发生在结果截断之前，实时与索引模式采用相同规则。不带查询的概览拒绝这些筛选选项。

`--trust` 接受 `unverified`、`machine-confirmed`、`human-reviewed`；`--expiry` 接受 `current`、`expired`、`unspecified`、`unknown`，同样支持重复取值。未声明有效期与日期无效分别为 `unspecified` 和 `unknown`。默认不按审核层级或有效期排除结果。

`--as-of TIMESTAMP` 只用于搜索和查询上下文，接受带 UTC 偏移的 RFC 3339 时间，默认使用请求时间。结果的 `data.evaluated_at` 记录归一化后的 UTC 时间；到达 `stale_after` 时即为过期。索引查询从已保存的原文按本次评估时间派生有效期，不沿用同步时的结论。它不改变观测时间、索引 generation 或缓存记录中的真实观测时间；未知元数据值的诊断仍保留。`--as-of` 不证明当时的源文件内容。

空查询或仅含空白的查询返回 `EMPTY_QUERY`（2）。查询按空白拆词，忽略大小写，并要求所有词都在卡片的 ID、标题、摘要、标签或类型字段值中出现。JSON 字段名、序列化转义和派生的审核、状态、有效期值不参与文本匹配；后者通过对应筛选参数选择。支持中文子串，结果按知识包引用排序。`context --query` 复用元数据检索，返回 `disclosure: metadata_candidates` 与 `read` 入口。

全文模式要求每个词出现在卡片或正文中，元数据命中每词计 5 分、仅正文命中每词计 1 分，同分按引用排序。它是确定性子串检索，未使用向量或分词模型。结果的 `snippet` 含原始字节范围与文本，普通片段最多 512 字节，正文上下文最多 4096 字节，均受整体条目预算约束。标题边界来自 Markdown 解析器，代码块中的 `#` 不作为标题。上下文从包含命中的最近章节开始，在下一个同级或上级标题前结束；标题距命中过远时从命中字节位置开始。每份文档只返回一个窗口，避免同一文档的重叠片段；不合并不同文件的来源。`coverage_gaps` 表达未选命中和未检查实时文件的事实。

`sync` 收录概念、`index.md` 和 `log.md`，不保存普通附件内容；只在完整、无结构错误的扫描后发布 generation。它不替代 `validate` 的质量警告、链接目标和可选附件/锚点检查；取消、预算不足或文件错误均保留旧索引。并发发布基于旧 generation 比较，冲突返回 `INDEX_CHANGED`（6）。`read --indexed` 的 cursor 绑定 generation；`--generation` 可在首个分页就拒绝过时的检索引用。索引视图的 `freshness` 为 `cached`，`checked_at` 为同步时间；本地文件索引仍是观测视图。`matches_binding_revision` 对 Git 表示索引提交是否等于当前绑定提交，对文件系统不证明源文件未变。

正文片段使用的标题清单每文件最多 4096 项；超过时返回 `SECTION_LIMIT` 和部分结果，片段仍保留精确字节定位，但章节边界可能不完整。

卡片中的 `trust`、`status` 和 `expiry` 从元数据派生，默认值与异常值的处理见 [派生与覆盖](#派生与覆盖)。这些字段表达源文件声明，不提供身份验证或内容真实性保证。

多包索引查询的 `--limit`、`--budget-bytes`、`--max-scan` 和 `--timeout-ms` 属于整个请求。按包名排序，将扫描配额轮流分配给仍有概念待评估的包；小包用不完的份额在读取前分给其他包。默认路径扫描模式在配额小于包内概念数时报告 `SCAN_LIMIT`，每包按路径顺序加载配额内的概念，再按统一得分和完整引用排序、应用全局结果及字节限制。配额分配不保证每包都有命中；故障或取消后的剩余份额不重新分配。

单包结果保留 `data.generation`；多包结果使用 `data.generations` 映射别名到版本，条目与 `next` 各自携带 generation。每项还包含 `bundle_id`。重复正文和冲突证据保持独立引用。各包是独立快照，不构成跨包同一时刻的快照。

多包查询中的缺失、损坏或读取期间变化的索引产生带包路径的诊断，保留其他包结果并返回退出码 7。无法读取的包列入 `coverage_gaps.unavailable_bundles`；任何包未完成评估时 `page.total` 为 null。有效包的观测包含 `scan_limit` 和 `indexed_documents_scanned`。单包请求仍直接返回对应错误；未知或有歧义的绑定在查询前拒绝。

`index status` 只查询索引元信息和统计，不验证正文完整性。`read --indexed` 在同一事务中读取版本与目标文档，仅验证该文档哈希；全文查询验证扫描配额内的正文。哈希不匹配返回 `INDEX_CORRUPT`，可重新执行 `sync NAME` 从源文件恢复正文；源内容未变时 generation 保持不变。

## 候选检索

`search --full-text` 与 `context --query --content` 接受 `--candidate-budget N`（0–1000000），显式启用 `candidates-v1`；省略时保留路径扫描。实时检索与概览拒绝该参数。候选阶段对卡片和正文的 Unicode 小写字符三元组求交，再按当前 `--as-of` 派生卡片并应用筛选；最终仍读取原文复核全部查询词、计算得分与片段。

`sync` 在正文发布事务内重建候选数据，每文档最多保存 4096 个不同三元组，每包最多 1000000 条倒排记录。超出容量的文档进入必查回退集合；没有三元组的短词查询扫描候选元数据。候选词仅来自约定的检索字段值和正文；有效期等筛选仍按本次评估时间派生，不加入检索词。

候选行预算计倒排路径、回退路径和卡片元数据读取，分别返回 `budget.candidate_rows_limit`、`candidate_rows_read`；正文仍受 `--max-scan` 限制。各包按名字顺序平均分配候选预算，余数依次分配，不在查询后重新分配。数据库清单计数、B-tree 查找和哈希计算不属于行预算，不能据此承诺总 I/O 或 CPU 上限。任何候选或正文截断返回 `CANDIDATE_LIMIT`、退出码 7 与未知总数；恰好读满候选预算时保守报告不完整。完整覆盖表示所选范围内查询候选已穷尽，不表示读取了每份正文。

派生数据当前使用独立版本 2（字段值检索），正文数据库仍为 v1，响应策略标识仍为 `candidates-v1`。升级前生成的版本 1 候选数据需要重新 `sync`；普通路径查询和索引原文读取不受影响。旧库可继续路径查询与原文读取，候选数据缺失返回 `INDEX_CANDIDATES_MISSING`（3），重新同步可建立。正文或派生表被发布事务之外的 SQL 写入时使候选清单失效；失效、版本或计数不符返回 `INDEX_CANDIDATES_CORRUPT`（5），包括范围外写入造成的全包失效。旧程序写入后也应重新同步。该机制检测一致性问题，不是对恶意数据库修改的身份认证。

多包候选查询发生读取错误时，故障包列入 `coverage_gaps.unavailable_bundles`。行数统计仅包含成功返回计量的包，是整个请求实际消耗的下界；预先分配的独立配额仍限制所有包的合计读取上限。

## 索引完整性检查

`index verify NAME` 使用只读数据库连接，在同一事务快照内检查所有类型的索引文档，包括保留文件。按路径检查合法性、正文 SHA-256 和文档 generation；完整扫描后，再从保存的路径、哈希和上游修订重算快照 generation。它不验证源文件新鲜度，也不替代 `validate` 的格式检查。

`data.documents_total` 为当前快照的文档数，`documents_checked` 为本次已检查数量，`complete` 表示检查是否完成。`valid` 只有在完整检查且无问题时为 true；已发现问题时为 false；未完成且尚无问题时为 null。`failures` 统计发现的问题数，一份文档可有多个问题；完整性问题最多保留 100 项诊断，超出时 `findings_truncated` 为 true，问题计数继续累加。扫描上限、截止时间或取消诊断另行保留。

存在候选清单时，还检查其版本、generation、行数，并在逐文档检查时从原文重算元数据与三元组；差异报告 `INDEX_CANDIDATES_CORRUPT`。即使正文扫描不完整，已发现清单失效时 `valid` 也为 false。旧库没有候选清单时只验证正文，不能据此推断候选查询可用。`sync` 同时修复正文与派生数据，失败或取消回滚整个发布事务。

完整且无问题返回 0，完整且发现损坏返回 5；扫描、64 MiB 检查预算或截止时间耗尽、取消时返回 7，保留已完成的检查结果，即使已发现损坏。单份正文超过 2 MiB 时报告 `INDEX_DOCUMENT_LIMIT`。其他完整性诊断包括 `INDEX_PATH_INVALID`、`INDEX_CORRUPT`、`INDEX_GENERATION_MISMATCH` 和 `INDEX_SNAPSHOT_MISMATCH`。缺失索引返回 3，且不创建正文数据库；不支持的数据库版本返回 `INDEX_VERSION`（2），不执行迁移。检查不会修复内容；显式 `sync` 在事务内重建该包的完整文档清单，包括移除多余记录。

`budget.scan_limit` 和 `indexed_documents_scanned` 记录文档扫描上限及实际数量，`index_bytes_limit` 和 `index_bytes_checked` 记录正文检查的累计字节上限及计数。`data.live_files_checked` 固定为 false。

## 故障与覆盖

浏览或检索遇到坏概念、文件读取失败时，返回带路径的诊断和部分结果，继续处理可用条目。多知识包概览和检索隔离知识包故障，保留其他包的结果；单知识包初始目录不可读时返回错误。未开始读取、探测失败或未能展开目录的包使用 `freshness: unknown`、`checked_at: null` 与未知覆盖状态。

实时读取命令会尝试打开 SQLite 观测缓存。消费命令遇到缓存故障时保留可读内容：写入失败使用 `CACHE_WRITE_FAILED`，打开失败且本次操作没有尝试写入卡片时使用 `CACHE_UNAVAILABLE`。索引命令使用独立数据库，不依赖观测缓存。`cache reset` 在不打开损坏数据库的情况下归档重建。配置写入使用系统锁，进程退出后自动释放；锁文件应保留。

文件系统失败的错误信息固定为英文，不含操作系统本地化文本：`message` 由稳定的错误类别与数值 `os error` 码组成，因此同一请求在中文或英文系统上返回相同文本，判断原因仍应使用 `code`。请求目标是目录时使用 `NOT_A_FILE`，不因平台差异表现为 `ACCESS_DENIED` 或 `NOT_FOUND`。

缓存问题与知识文件覆盖分别表达，完整校验发现的文档错误优先于缓存诊断。`SCAN_LIMIT` 仅用于扫描预算耗尽，文件故障或取消不会被标为预算不足。

`validate` 返回 `data.link_check`，其中 `scope` 为 `bundle_markdown_targets`，`complete` 为布尔值。它说明包内 Markdown 链接与目录目标检查是否完成；扫描未完成时跳过目标存在性判断，读取失败、链接预算耗尽、正文收集或警告截断，以及取消都会使其为 false。它不表示目标全部有效或关系图已建立，检查范围见 [正文链接与来源脚注](#正文链接与来源脚注)。`link_check.external_links` 为确定性统计的外部链接（HTTP(S) 与其他 URI 方案）数量，不访问、不校验外部目标。

`budget.link_limit` 与 `link_bytes_limit` 给出待检查链接的数量、字符串字节上限；`links_checked` 是实际检查的链接数，`link_bytes_used` 是保留链接占用的字符串字节数。`LINK_LIMIT` 返回退出码 7，链接阶段取消或预算耗尽均保留已完成的元数据覆盖。具体上限见 [诊断上限](#诊断上限)。

计算资源检查独立返回 `data.resource_check`，范围为 `bundle_computation_resources`。缺失文件或目录类型不符产生字段级警告；权限、越界、查询失败使用操作诊断和部分结果。查询预算耗尽为 `RESOURCE_LIMIT`（7），不降低已完成的元数据扫描覆盖。外部 URI 仅计入 `external_skipped`，不访问。`resources_checked`、`resource_limit`、`resource_bytes_used`、`resource_bytes_limit` 给出预算。

## JSON 响应

[响应 Schema](../schemas/response.schema.json) 定义 envelope 和部分业务字段约束；各命令字段语义仍以本文及实际响应为准，不能将 Schema 通过等同于全部业务语义正确。

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

`search` 在 `ok` 或 `partial` 状态下必须返回 `data.mode`，取值为 `metadata` 或 `full_text`。元数据模式还必须返回非空字符串 `query`、字符串 `evaluated_at` 和数组 `items`；无命中或尚未获得结果时数组可为空。响应 Schema 对每个元数据结果要求以下字段：

| 字段 | 结构 |
| --- | --- |
| `id`、`type`、`title`、`status`、`bundle_id` | 字符串；`type` 非空，未知概念类型和声明状态继续保留 |
| `kind` | 固定为 `concept` |
| `ref` | `@别名/概念ID` 形式的字符串 |
| `description` | 字符串或 null，不省略 |
| `tags` | 字符串数组，可为空 |
| `trust` | `unverified`、`machine-confirmed` 或 `human-reviewed` |
| `expiry` | `current`、`expired`、`unspecified` 或 `unknown` |
| `content_sha256` | 64 位小写十六进制 SHA-256 |
| `upstream_revision` | Git 提交字符串或 null，不省略 |

元数据结果与 `data` 继续允许扩展字段。错误响应不要求成功结果字段；全文条目和其他命令不套用这组元数据卡片约束。Schema 检查字段结构，不证明引用目标存在、哈希对应当前文件或结果语义正确。

文件系统读取视图为 `observed`，上游修订为 null；Git 视图为 `snapshot`，`upstream_revision` 是完整 commit ID。Git 原文、卡片与 cursor 均携带或绑定提交。`metadata` 覆盖状态必须结合 `coverage.scope` 解读：单文件、单目录与全知识包扫描不是同一范围。实时命令的 `body`、`graph` 是保守覆盖声明；索引的完整正文覆盖仅描述已发布的索引版本。

`page.total` 可以为 null；`has_more=false` 只说明该操作没有续页，不证明全库扫描完整。卡片返回也不意味着 Agent 已读取正文。

`diagnostics[].severity` 为 `error` 或 `warning`。元数据及正文警告的 `path` 为完整文件引用，如 `@handbook/services/auth.md`；元数据的 `field` 为 frontmatter 的 JSON Pointer，如 `/verified/0/at`。正文、操作或文档级诊断的 `field` 为 null，包括截断提示。默认情况下，警告不改变执行状态或元数据覆盖；显式启用质量门槛时按下文规则判定退出码。诊断条数规则见 [诊断上限](#诊断上限)。

正文诊断通过可选的 `location: {byte_start, byte_end}` 标识完整原文件中的 UTF-8 字节半开区间，包含 BOM、frontmatter 与换行所占字节；它不相对于当前读取分页。没有具体正文位置的诊断省略此字段。

缺少内联计算块时，`location` 指向已有的计算章节标题；计算章节也缺失时，使用正文起点的零长度区间表示应补充内容的位置。计算合同的字段与正文规则见 [计算合同与正文](#计算合同与正文)。

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

`validate` 返回 `data.failures` 与 `data.warnings`，分别统计失败文件与返回的元数据、正文及资源警告。未启用质量门槛时，只有警告且检查完成仍为 `ok`、退出码 0。

显式指定 `validate --max-warnings N`（允许 0）启用质量门槛。`data.quality_gate` 返回 `max_warnings`、`warnings_observed`、`warnings_total`、`complete` 和 `passed`。观测数不包含诊断截断提示；完整时总数等于观测数，不完整时总数为 null。已观测数量超过门槛即确定 `passed: false`；未超过且统计不完整时为 null，只有完整且未超过时才为 true。

扫描、链接或资源预算耗尽，读取失败，诊断截断，以及已启用检查中的未知目标都会阻止门槛通过。完整检查超限返回 `WARNING_LIMIT_EXCEEDED`、退出码 1；不完整检查优先保持退出码 7，只有警告截断等导致计数不完整而此前尚未标记部分结果时增加 `QUALITY_GATE_INCOMPLETE`。已确定的结构错误保留退出码 1；缓存等操作错误不会被质量门槛覆盖。即使 `passed: true`，仍需检查响应执行状态。默认未指定门槛时行为不变。

Agent 应读取稳定的诊断 code，避免匹配错误文本；只引用实际读取的文件、字节范围和内容哈希，并保留影响答案的过期、审核和覆盖限定。消费流程见 [消费 Skill](../skills/okf-cli/SKILL.md)。

## OKF 检查矩阵

本节定义结构检查、元数据与正文诊断，依据为固定提交的 [OKF v0.2 规范](https://github.com/GoogleCloudPlatform/open-knowledge-format/blob/ad30107c31c06aec8a7d5636e0d1058118604e6f/SPEC.md)。命令与响应规则见前文的 [命令](#命令) 和 [JSON 响应](#json-响应)。

### 诊断与消费

`read`、`browse`、`search`、`context --query` 和 `validate` 共用元数据与正文诊断规则。实时文件中的包内链接目标存在性与类型由 `validate` 检查；`neighbors` 另报告已同步索引中的目标存在性。

- `error` 表示结构不合规或操作故障，是否终止请求由命令决定。例如浏览跳过坏概念并返回部分结果，显式 `read --raw` 可以返回带结构错误诊断的原文。
- `warning` 表示元数据或正文中的质量建议，包括来源归因问题与断链。警告不拒绝概念，也不降低元数据覆盖；默认不改变退出码，显式启用 `--max-warnings` 时按 [质量门槛](cli-contract.md#退出码) 判定。
- 可选字段缺失、未知 `type`、未知扩展字段和缺失索引不会触发元数据警告。未知字段保持原值，原文不被重写。

诊断字段定义见 [JSON 响应](cli-contract.md#json-响应)。下表中的字段级警告通过 `field` 定位，例如 `/verified/0/at`。

### 检查规则

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

### 派生与覆盖

卡片只从有效 Actor 标识派生审核层级；无有效核验者时为 `unverified`，人类核验者优先于机器核验者。缺少或非字符串 `status` 派生为 `stable`；未知字符串状态原样保留并附警告。非法 `stale_after` 派生为 `unknown`，到期判断使用 `now >= stale_after`。

### 正文链接与来源脚注

`validate` 对 `sources[].resource` 按材料标识检查，不会触发网络访问或文件存在性检查。来源归因通过脚注标签与 `sources[].id` 关联，不依赖来源在列表中的位置；来源无需全部被引用。

使用 Markdown 解析器提取普通链接、引用式链接、图片引用与脚注，排除代码和 HTML 块中的链接。图片不构成关系导航边；图片附件目标检查由 `validate --attachments` 显式启用。包根路径以 `/` 开头，文档相对路径允许 `.` 和 `..`，归一化后必须留在知识包内。路径中的百分号编码按 UTF-8 解码；查询串和片段不参与文件定位。外部 URI 不触发访问，默认保留片段但不检查锚点；显式启用 `--anchor-style headings-v1` 后按下述 CLI 扩展规则检查，不代表所有渲染器。

目录形式包括解码后以 `/` 结尾的路径，以及末段为 `.` 或 `..` 的导航。文件链接与目录链接分别匹配相应类型的目标。Markdown 脚注定义的匹配可以忽略大小写，来源 ID 则按引用标签精确关联。

| 检查对象 | 诊断 code | 行为 |
| --- | --- | --- |
| 非法路径字符或编码 | `INVALID_LINK_PATH` | 当前文件返回正文警告 |
| 归一化后越出知识包 | `LINK_PATH_ESCAPE` | 当前文件返回正文警告，不读取目标 |
| 脚注没有匹配的来源 ID | `UNRESOLVED_SOURCE` | 当前概念返回正文警告 |
| 脚注引用缺少定义 | `UNDEFINED_FOOTNOTE` | 当前概念返回正文警告 |
| 包内 Markdown 文件或显式目录不存在，或文件与目录类型不符 | `BROKEN_LINK` | 仅在 `validate` 完整列举后检查 |

按需读取仅检查当前文件，不访问链接目标；`validate` 复用完整目录清单检查目标。保留文件参与链接检查，但不要求来源 frontmatter。正文位置的响应格式见 [JSON 响应](cli-contract.md#json-响应)。

### 计算合同与正文

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

### 诊断上限

| 范围 | 上限 | 超出时的诊断 | 影响 |
| --- | --- | --- | --- |
| 每份概念的元数据警告 | 64 条 | `METADATA_DIAGNOSTICS_TRUNCATED` | 追加一条截断警告，元数据覆盖保持不变 |
| 每份文件的链接与脚注引用收集 | 各 256 个 | `BODY_DIAGNOSTICS_TRUNCATED` | 追加一条截断警告，链接检查完整性为 false |
| 每份文件的正文警告 | 64 条，路径、脚注、计算正文与断链共用 | `BODY_DIAGNOSTICS_TRUNCATED` | 每份文件最多追加一条截断警告，链接检查完整性为 false |
| `validate` 保留的待检查链接 | 4096 条或 2 MiB 字符串总字节 | `LINK_LIMIT` | 返回部分结果与退出码 7，保留已完成的元数据覆盖 |

元数据和正文的警告上限分别计数；截断提示不占各自的 64 条限额。整个响应还受输出字节上限约束，`validate` 的预算字段与检查完整性见 [CLI 参考](cli-contract.md#故障与覆盖)。

### 范围与验证

扫描未完成时不推断目标缺失；文件读取失败或正文诊断截断使链接检查完整性为 false。链接检查阶段取消也保留已完成的元数据覆盖。尚未覆盖的能力见 [已知边界](architecture.md#已知边界)。

[固定样例](../crates/okf-core/tests/fixtures/metadata-cases.json) 记录规范版本、输入元数据、预期诊断和派生结果。元数据验证应覆盖字段形状、单对象与列表等价、时区与到期边界，以及警告与结构错误并存。

[正文测试](../crates/okf-core/tests/markdown.rs) 覆盖路径解析、引用式链接、代码排除与来源 ID 关联。CLI 验收还应检查文件与目录类型、混合警告共用上限、扫描及链接检查阶段取消，以及分页中的诊断位置始终相对于原文件。读取前后核对源文件字节与哈希。

[计算合同样例](../crates/okf-core/tests/fixtures/computation-cases.json) 覆盖合同字段、未知 runtime、零参数、内联与文件形式、章节边界、标题可见文本及空正文定位。核对各消费命令返回相同诊断，且警告不影响消费、元数据覆盖和原文字节；正文与元数据警告分别验证共享上限。

可使用 [okf-skills 校验器](https://github.com/scaccogatto/okf-skills/blob/49f8fb4987989b7a443ce278c6aa68ba0493b51f/skills/validate/scripts/okf_validate.py) 交叉核对默认模式下的结构合规结果。参考工具会提示部分推荐字段缺失，并采用不同的时间、来源和链接建议策略；本项目的警告策略以本矩阵为准，不要求警告条数一致。

### 可选附件与锚点检查

`validate --attachments` 增加普通正文链接与图片引用的附件元信息检查，返回 `MISSING_ATTACHMENT`/`MISSING_IMAGE` 字节位置警告，不读取附件内容；`attachment_check` 报告 `images_included`（恒为 `true`）、`images_checked`（图片引用数量）与 `types`（检查目标的小写扩展名集合，去重、升序，图片与普通附件一并计入）。`--anchor-style headings-v1` 增加显式规则下的 `MISSING_ANCHOR` 警告，规则与覆盖说明见 [CLI 参考](cli-contract.md#关系导航与扩展检查)。这两项均不增加 OKF 必填要求，质量警告不改变文档结构合规性。

附件与锚点结果分别返回 `attachment_check`、`anchor_check`，不替代原有 `link_check` 和计算 `resource_check`。完整列举前不判断链接目标缺失；附件只查询元信息，锚点复用本次已读取文档的标题清单。显式 HTML `id`/`name` 锚点的属性值先解码字符引用（如 `&amp;`、`&#45;`、`&#x2D;`）再匹配；无法识别的自定义 HTML 锚点保留为未检查。
