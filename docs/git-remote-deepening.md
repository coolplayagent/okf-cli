# Git 远端深化 — 开发文档（okf-cli 实施计划 ③）

> 状态：设计稿，待评审（评审通过后再进入实现与提交）。
> 范围：roadmap「3. Git 远端深化」三项子能力。
> 语言：Rust（与仓库一致；Python 仅用于 `scripts/` 交付校验，不参与 CLI 运行时）。

---

## 1. 目标

在现有「公开 HTTPS Git 知识包只读消费」基础上，补齐三块能力：

1. **私有凭据引用**：支持私有 HTTPS 仓库，凭据以「引用」方式提供，绝不落盘、不进配置、不进响应。
2. **精确传输限制**：把当前「采样磁盘配额」升级为可承诺的精确字节上限。
3. **历史对象过期策略**：可回收历史/不可达对象，且不破坏仍被绑定引用的提交。

**验收总纲**（来自 roadmap）：
- 不泄露凭据；失败保留绑定；清理不影响仍使用的提交。
- 现有对象预算为采样检查，不作精确网络字节承诺；当前压缩保留历史、不承担版本回收。

---

## 2. 现状梳理

### 2.1 数据模型（`crates/okf-application/src/lib.rs`）

```rust
pub struct BundleBinding { id, name, kind, root, scope, git: Option<GitRevision> }
pub struct GitRevision {
    pub reference: String,
    pub commit: String,
    pub bundle_path: String,
    pub remote: Option<String>,   // 已预留，当前恒 None
}
```

`remote` 字段已存在但未使用——本次将真正启用，并新增凭据引用字段。

### 2.2 适配器（`crates/okf-adapters/src/git.rs`，827 行）

- `command()`：剥离所有 `GIT_*` 环境变量；`GIT_CONFIG_NOSYSTEM=1`；`GIT_TERMINAL_PROMPT=0`；`credential.helper=`（置空）；`http.followRedirects=false`；`protocol.https.allow=always`；进程组/JobObject 隔离。
- `fetch_bounded()`：校验 remote 为 `https://` 且**不含 `@`**（拒绝凭据内嵌）；`fetch --depth=1 --no-tags ... +REF:refs/okf/fetch-{pid}-{nanos}`。
- `check_storage()`：采样遍历 `objects/` 目录求和文件大小，fetch 前/每 50ms/后各查一次；超限返回 `GIT_STORAGE_LIMIT`。
- `cache(compact)`：`repack -a -d --keep-unreachable`，保留历史与不可达对象。
- `transport_error()`：把 stderr 映射为 `GIT_AUTH_FAILED / GIT_TLS_FAILED / GIT_REMOTE_NOT_FOUND / GIT_TRANSPORT_FAILED`，**原始 stderr 不进响应**（凭据安全的关键）。
- `lock()`：`okf-cache.guard` 系统文件锁，跨进程互斥写。

### 2.3 契约（`docs/cli-contract.md` §Git 远端与缓存）

- `--max-git-bytes` 默认 256 MiB = 「托管仓库对象目录总占用预算」，明示「不是精确网络字节上限」。
- `bundle cache NAME [--compact]`：查看/压缩，保留历史引用及不可达对象，不做历史过期。
- 失败保留旧绑定；`GIT_STORAGE_LIMIT`(7)、`GIT_CACHE_LOCKED`(5)。

---

## 3. 子任务 A：私有凭据引用

### 3.1 设计原则

凭据只以**引用**存在，运行时解析后**仅注入 git 子进程环境**，全程满足：

1. 不进 `config.toml`（只存引用字符串）；
2. 不进命令行参数（`ps` 可见）；→ 用 git 的 `GIT_CONFIG_KEY_n` / `GIT_CONFIG_VALUE_n` 环境变量注入；
3. 不进 stdout / stderr / 响应 envelope；→ 沿用现有 stderr 不回传机制；
4. 文件类凭据拒绝权限过宽（Unix 要求 `0600`）。

### 3.2 配置扩展

`GitRevision` 新增字段（`skip_serializing_if = "Option::is_none"`）：

```rust
pub struct GitRevision {
    pub reference: String,
    pub commit: String,
    pub bundle_path: String,
    pub remote: Option<String>,
    pub credential: Option<CredentialRef>,   // 新增
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind")]
pub enum CredentialRef {
    Env { var: String },          // env: 从环境变量读取 token
    File { path: String },        // file: 从受限文件读取 token
    Helper { name: String },      // helper: 委托 git credential helper
}
```

CLI 契约（`bundle add`）：

```
bundle add NAME --kind git --url HTTPS_URL [--ref REF] [--credential env:VAR | file:PATH | helper:NAME]
```

`bundle refresh` 复用绑定内已存的 `credential` 引用（不重新输入秘密）。

### 3.3 凭据注入机制

`fetch` 子进程构建时（`command()` 内部，剥离 `GIT_*` 之后）：

1. 解析 `CredentialRef`：
   - `Env{var}` → `std::env::var(var)`，空值报 `GIT_CREDENTIAL_MISSING`；
   - `File{path}` → 读文件（校验 Unix 权限 `0o600`），trim 空白；
   - `Helper{name}` → 不读秘密，改设 `credential.helper=<name>`（本迭代可先只做 env/file，helper 作为后续）。
2. 用环境变量注入（不进 argv）：
   ```
   GIT_CONFIG_COUNT=2
   GIT_CONFIG_KEY_0=http.extraHeader
   GIT_CONFIG_VALUE_0=Authorization: Bearer <token>
   GIT_CONFIG_KEY_1=http.extraHeader
   GIT_CONFIG_VALUE_1=Authorization: Basic <base64>   // 按 remote 主机选择 Bearer 或 Basic
   ```
   或对 GitHub 风格 PAT 使用 `url.<base>.insteadOf` + `https://x-access-token:<token>@...` 的等价注入，**同样走环境变量，绝不出现在 argv**。

3. 失败语义：解析失败/缺凭据返回新错误码 `GIT_CREDENTIAL_MISSING`（退出码 5），**保留旧绑定**；网络 401/403 仍走 `GIT_AUTH_FAILED`。

### 3.4 安全加固

- 新增单测：断言凭据字符串**不出现**在 argv、stdout、stderr、响应、`config.toml` 五处。
- `bundle status` / `bundle cache` 不序列化 `credential` 的明文（引用可序列化，秘密不可）。
- 日志/诊断沿用「stderr 不回传」；任何 error message 不得拼接 token 值。

---

## 4. 子任务 B：精确传输限制

### 4.1 目标

在保留「存储占用预算（采样）」之外，新增**精确传输字节上限**，并给出可承诺的计量语义。

### 4.2 语义定义（关键：先明确"精确"指什么）

- `--max-git-bytes`（现有）：**存储占用**，采样，继续保留——保护磁盘，不承诺网络字节。
- `--max-fetch-bytes`（新增）：**单次 fetch 实际接收对象字节**，精确承诺。

精确计量方式（git 不直接暴露 wire 字节，采用**对象字节精确核算**）：

1. fetch 前快照：`git cat-file --batch-all-objects --batch-check='%(objectname) %(objectsize)'` 得现存对象集合；
2. fetch 后同样快照，**求差集** = 本次新接收对象，求和 = 精确传输对象字节数；
3. 差集字节 > `--max-fetch-bytes` 时：**拒绝发布新绑定**（保留旧绑定），并可选回滚本次 fetch 写入的临时 ref。

> 说明：`--max-fetch-bytes` 承诺「新接收对象的原始对象字节和」，不等同 wire 层 TLS/压缩开销；文档中明确此边界（与 roadmap「不作精确网络字节承诺」的既有措辞对齐，本次升级为「精确对象字节承诺」）。

### 4.3 实现要点

- 新函数 `precise_received_bytes(root, before: HashSet<oid>, control) -> Result<u64>`。
- 复用现有 `check_storage` 的 50ms 检查做「软」限流（防单次暴涨），精确核算做「硬」判定。
- 超限错误码沿用 `GIT_STORAGE_LIMIT`(7) 或新增 `GIT_FETCH_LIMIT`，统一保留旧绑定语义。
- `bundle cache` 报告里补充 `last_fetch_bytes` 供观测。

---

## 5. 子任务 C：历史对象过期策略

### 5.1 目标

提供显式的历史/不可达对象回收，且**不破坏任何仍被绑定引用的提交**。

### 5.2 设计

新增 `bundle cache NAME --gc`（或独立 `bundle gc NAME`）：

1. 收集「存活提交」：遍历 project + user 作用域所有 `BundleBinding`，取每个 `git.commit` 作为存活集；
2. 为每个存活提交建立临时 pin ref：`refs/okf/pinned/<hash>`；
3. 执行回收：`git gc --prune=now`（此时 `refs/okf/pinned/*` 保证存活对象可达，其余不可达对象被回收）；
4. 清理临时 pin ref 与历史 `refs/okf/fetch-*` 陈旧 ref；
5. 返回回收前后 `object_bytes`（复用 `check_storage`）。

### 5.3 安全约束（对齐验收）

- 存活集必须**跨作用域**收集（project + user），不能只看当前绑定。
- 回收前二次校验：存活 commit 在回收后仍可 `cat-file -e`。
- `--gc` 与 `--compact`、`fetch`、`refresh` 共用 `lock()` 互斥。
- 默认**不自动过期**；只提供显式命令，避免误删。

---

## 6. 模块改动清单

| 文件 | 改动 |
| --- | --- |
| `crates/okf-application/src/lib.rs` | `GitRevision` 增 `credential`；`CredentialRef` 枚举；序列化规则 |
| `crates/okf-application/src/service.rs` | 透传 credential 引用；`bundle add/refresh/cache` 用例装配 |
| `crates/okf-adapters/src/git.rs` | A：凭据解析+环境注入；B：`precise_received_bytes`；C：`gc` 实现、pin ref 管理 |
| `crates/okf-adapters/src/config.rs` | 配置读写 `credential`（引用）字段 |
| `apps/okf-cli/src/*` | 新增 `--credential`、`--max-fetch-bytes`、`--gc` 参数解析与校验 |
| `schemas/response.schema.json` | 新增/扩展错误码、`bundle cache` 响应字段 |
| `docs/cli-contract.md` | 契约更新（凭据引用、精确字节语义、gc） |
| `docs/roadmap.md` / `implementation-status.md` | 状态推进 |
| `skills/okf-cli/SKILL.md` | 若有涉及消费流程，同步说明（预计无大改） |

---

## 7. 测试与验收

- **单元**：`CredentialRef` 解析/校验；凭据不进 argv/stdout/stderr/config 的回归断言；`precise_received_bytes` 差集核算；`gc` 存活集收集与回收后可达性。
- **集成**：本地 HTTPS git 服务注入 401/403（断言 `GIT_AUTH_FAILED` + 凭据不泄露）；私有仓库（env/file 凭据）成功 fetch + refresh；`--max-fetch-bytes` 超限保留旧绑定；`--gc` 回收后旧 commit 仍可读、`fetch-*` 陈旧 ref 被清理。
- **验收**（对齐 roadmap）：
  - 凭据：五处不泄露；失败保留绑定；
  - 精确字节：差集核算可复现、语义文档化；
  - 过期：清理不影响存活提交；默认不自动过期。
- **CI**：三平台（Linux/macOS/Windows GNU+MSVC）`cargo test --workspace --all-targets --locked`、`clippy -D warnings`、`fmt --check` 通过；`scripts/check_delivery.py` 全绿。

---

## 8. 提交规范

- 分支：`feat/git-remote-deepening`（或按三项拆三支，评审后合并）。
- 提交粒度：按 A / B / C 三个子任务各一个（或更细）提交；每个提交自包含（测试同 commit）。
- 提交信息遵循仓库既有风格（`类型: 描述`），例如：
  - `feat(git): private credential reference via env/file without leaking secrets`
  - `feat(git): precise fetch byte accounting`
  - `feat(git): explicit history object gc`
- 推送：需直连 github.com（镜像只读）；凭据仅走环境变量，**严禁提交任何 token/密钥**。

---

## 9. 分阶段实施计划

| 阶段 | 内容 | 交付 |
| --- | --- | --- |
| P0 | A：私有凭据引用（env/file） | 单测 + 集成 + 契约 |
| P1 | B：精确传输限制 | 差集核算 + 契约 |
| P2 | C：历史对象过期 | gc + 安全约束 |
| P3 | 三平台 CI + 交付校验 + 文档收口 | 全绿 + 文档更新 |

---

## 10. 待确认（评审点）

1. `credential` 引用三态（env/file/helper）首期是否只做 `env` + `file`，`helper` 留待后续？
2. 精确字节语义：是否接受「对象字节和」而非「wire 字节」？（git 无法廉价给出 wire 字节）
3. 历史过期：是否接受「仅显式命令、不自动过期」的安全默认？
