use clap::{Args, Parser, Subcommand, ValueEnum};
use okf_adapters::{default_home, ConfigRegistry, Git, Readers, SqliteCache, SqliteIndex};
use okf_application::{
    App, BundleBinding, BundleReader, BundleRegistry, Cache, Index, ReadOptions, RequestControl,
    Response, UnavailableCache,
};
use okf_core::{sha256, Diagnostic, Error, Result};
use serde_json::json;
use std::{
    io::Write,
    path::PathBuf,
    process::ExitCode,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

#[derive(Parser)]
#[command(
    name = "okf",
    version,
    about = "Read OKF knowledge through bounded, progressive CLI disclosure"
)]
struct Cli {
    #[arg(long, global = true, value_enum, default_value = "text")]
    format: Format,
    #[arg(long, global = true, value_enum)]
    scope: Option<Scope>,
    /// Absolute runtime directory; overrides OKF_HOME.
    #[arg(long, global = true)]
    home: Option<PathBuf>,
    #[arg(long, global = true, default_value_t = 131072, value_parser = parse_output)]
    max_output_bytes: usize,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Text,
    Json,
}
#[derive(Clone, Copy, ValueEnum)]
enum Scope {
    Project,
    User,
}
impl Scope {
    fn label(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::User => "user",
        }
    }
}

#[derive(Subcommand)]
enum Command {
    #[command(name = "bundle")]
    Bundle {
        #[command(subcommand)]
        command: BundleCommand,
    },
    /// Discover registered knowledge bundles without loading concept bodies.
    Context {
        #[arg(long = "bundle")]
        bundles: Vec<String>,
        #[arg(long)]
        query: Option<String>,
        /// Select original body excerpts from synchronized indexes (requires --bundle).
        #[arg(long, requires = "query", requires = "bundles")]
        content: bool,
        #[command(flatten)]
        page: ContextArgs,
        #[command(flatten)]
        filters: FilterArgs,
    },
    /// Expand one directory. Reserved index text is available through read.
    Browse {
        reference: String,
        #[command(flatten)]
        page: BrowseArgs,
    },
    /// Read exact original UTF-8 bytes, with revision-checked continuation.
    Read {
        reference: String,
        #[arg(long)]
        raw: bool,
        /// Read the explicitly synchronized offline index.
        #[arg(long, conflicts_with = "raw")]
        indexed: bool,
        /// Require the index generation returned by search/context.
        #[arg(long, requires = "indexed")]
        generation: Option<String>,
        #[command(flatten)]
        page: ReadArgs,
    },
    /// Search live metadata, or synchronized bodies with --full-text.
    Search {
        query: String,
        #[arg(long = "bundle")]
        bundles: Vec<String>,
        #[arg(long, requires = "bundles")]
        full_text: bool,
        #[command(flatten)]
        page: SearchArgs,
        #[command(flatten)]
        filters: FilterArgs,
    },
    Status {
        name: Option<String>,
    },
    /// Read evidenced incoming/outgoing links from a synchronized index.
    Neighbors {
        reference: String,
        #[arg(long, default_value = "both", value_parser = ["both", "incoming", "outgoing"])]
        direction: String,
        #[command(flatten)]
        page: SearchArgs,
    },
    Validate {
        name: String,
        /// Maximum collected computation resource declarations (metadata queries only).
        #[arg(long, default_value_t = 4096, value_parser = parse_scan)]
        max_resources: usize,
        /// Check non-Markdown body-link targets and images using metadata only.
        #[arg(long)]
        attachments: bool,
        /// Opt in to a deterministic heading-ID profile; custom HTML remains unknown.
        #[arg(long, value_parser = ["headings-v1"])]
        anchor_style: Option<String>,
        /// Fail the quality gate when warnings exceed this count (default: no gate).
        #[arg(long)]
        max_warnings: Option<u64>,
        #[command(flatten)]
        page: ScanArgs,
    },
    Doctor,
    /// Read embedded build provenance without opening configuration or caches.
    BuildInfo,
    /// Atomically publish a complete bounded scan into the offline content index.
    Sync {
        name: String,
        #[command(flatten)]
        page: ScanArgs,
    },
    Index {
        #[command(subcommand)]
        command: IndexCommand,
    },
    /// Archive and recreate the disposable observation cache.
    Cache {
        #[command(subcommand)]
        command: CacheCommand,
    },
    Skill {
        #[command(subcommand)]
        command: SkillCommand,
    },
}

#[derive(Subcommand)]
enum BundleCommand {
    Add {
        name: String,
        #[arg(long, default_value = "fs")]
        kind: String,
        #[arg(long)]
        path: Option<PathBuf>,
        #[arg(long, conflicts_with = "path")]
        url: Option<String>,
        #[arg(long = "ref")]
        reference: Option<String>,
        #[arg(long)]
        bundle_path: Option<String>,
        /// Sampled object-cache byte budget for public HTTPS fetches.
        #[arg(long)]
        max_git_bytes: Option<u64>,
        #[command(flatten)]
        work: WorkArgs,
    },
    List,
    Show {
        name: String,
    },
    Remove {
        name: String,
    },
    /// Resolve the configured Git ref and atomically publish its new commit.
    Refresh {
        name: String,
        #[arg(long, default_value_t = 268435456)]
        max_git_bytes: u64,
        #[command(flatten)]
        work: WorkArgs,
    },
    /// Inspect or compact a managed remote object cache, retaining historical objects.
    Cache {
        name: String,
        #[arg(long)]
        compact: bool,
        #[command(flatten)]
        work: WorkArgs,
    },
}

#[derive(Subcommand)]
enum SkillCommand {
    /// Install the bundled CLI consumption skill. Existing files are preserved.
    Install { path: PathBuf },
}

#[derive(Subcommand)]
enum CacheCommand {
    Reset,
}
#[derive(Subcommand)]
enum IndexCommand {
    Status {
        name: String,
    },
    Clear {
        name: String,
    },
    /// Check stored content and snapshot integrity without changing the index.
    Verify {
        name: String,
        /// Maximum stored documents to check, including reserved files.
        #[arg(long, default_value_t = 100, value_parser = parse_scan)]
        max_scan: usize,
        #[command(flatten)]
        work: WorkArgs,
    },
}

#[derive(Args, Default)]
struct FilterArgs {
    /// Opt in to indexed candidates with a shared row budget (0..1000000).
    #[arg(long, value_parser = clap::value_parser!(u32).range(0..=1000000))]
    candidate_budget: Option<u32>,
    /// Search this bundle-relative directory and its descendants.
    #[arg(long = "dir", value_parser = parse_directory)]
    directory: Option<String>,
    /// Exact type match; repeat for alternatives.
    #[arg(long = "type")]
    types: Vec<String>,
    /// Exact tag match; repeat for alternatives.
    #[arg(long = "tag")]
    tags: Vec<String>,
    /// Exact lifecycle status match; repeat for alternatives.
    #[arg(long = "status")]
    statuses: Vec<String>,
    /// Exact derived review tier; repeat for alternatives.
    #[arg(long = "trust", value_parser = ["unverified", "machine-confirmed", "human-reviewed"])]
    trusts: Vec<String>,
    /// Declared expiry state; repeat for alternatives.
    #[arg(long = "expiry", value_parser = ["current", "expired", "unspecified", "unknown"])]
    expiries: Vec<String>,
    /// Evaluate lifecycle at this RFC 3339 timestamp with an explicit UTC offset.
    #[arg(long, value_parser = parse_as_of)]
    as_of: Option<chrono::DateTime<chrono::Utc>>,
}
impl FilterArgs {
    fn value(&self) -> okf_application::SearchFilter {
        okf_application::SearchFilter {
            candidate_budget: self.candidate_budget.map(|v| v as usize),
            directory: self.directory.clone(),
            types: self.types.clone(),
            tags: self.tags.clone(),
            statuses: self.statuses.clone(),
            trusts: self.trusts.clone(),
            expiries: self.expiries.clone(),
            as_of: self.as_of,
        }
    }
    fn is_empty(&self) -> bool {
        self.directory.is_none()
            && self.candidate_budget.is_none()
            && self.types.is_empty()
            && self.tags.is_empty()
            && self.statuses.is_empty()
            && self.trusts.is_empty()
            && self.expiries.is_empty()
            && self.as_of.is_none()
    }
}

fn parse_as_of(value: &str) -> std::result::Result<chrono::DateTime<chrono::Utc>, String> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|time| time.with_timezone(&chrono::Utc))
        .map_err(|_| "expected an RFC 3339 timestamp with an explicit UTC offset".into())
}

fn parse_directory(value: &str) -> std::result::Result<String, String> {
    let directory = value.strip_suffix('/').unwrap_or(value);
    if directory.is_empty() {
        return Err("expected a bundle-relative directory; omit --dir for the root".into());
    }
    okf_core::logical_path(directory).map_err(|error| error.to_string())?;
    Ok(directory.to_owned())
}

#[derive(Args)]
struct ResultArgs {
    #[arg(long, default_value_t = 20, value_parser = parse_limit)]
    limit: usize,
    #[arg(long, value_parser = parse_budget)]
    budget_bytes: Option<usize>,
}
impl ResultArgs {
    fn options(
        &self,
        default_budget: usize,
        max_scan: usize,
        cursor: Option<String>,
    ) -> ReadOptions {
        ReadOptions {
            limit: self.limit,
            budget: self.budget_bytes.unwrap_or(default_budget),
            max_scan,
            cursor,
        }
    }
}

#[derive(Args)]
struct WorkArgs {
    /// Cooperative elapsed-time limit for this request, in milliseconds.
    #[arg(long, default_value_t = 30_000, value_parser = clap::value_parser!(u64).range(1..=3_600_000))]
    timeout_ms: u64,
}

#[derive(Args)]
struct ScanArgs {
    /// Maximum directory opens and iterator steps, including ignored entries.
    #[arg(long, default_value_t = 100, value_parser = parse_scan)]
    max_scan: usize,
    #[command(flatten)]
    work: WorkArgs,
}

#[derive(Args)]
struct ContextArgs {
    #[command(flatten)]
    results: ResultArgs,
    /// Maximum directory opens and iterator steps; requires --query.
    #[arg(long, requires = "query", value_parser = parse_scan)]
    max_scan: Option<usize>,
    #[command(flatten)]
    work: WorkArgs,
}
impl ContextArgs {
    fn options(&self) -> ReadOptions {
        self.results
            .options(24 * 1024, self.max_scan.unwrap_or(100), None)
    }
}

#[derive(Args)]
struct SearchArgs {
    #[command(flatten)]
    results: ResultArgs,
    #[command(flatten)]
    scan: ScanArgs,
}
impl SearchArgs {
    fn options(&self) -> ReadOptions {
        self.results.options(16 * 1024, self.scan.max_scan, None)
    }
}

#[derive(Args)]
struct BrowseArgs {
    #[command(flatten)]
    search: SearchArgs,
    #[arg(long)]
    cursor: Option<String>,
}
impl BrowseArgs {
    fn options(&self) -> ReadOptions {
        self.search
            .results
            .options(16 * 1024, self.search.scan.max_scan, self.cursor.clone())
    }
}

#[derive(Args)]
struct ReadArgs {
    #[arg(long, default_value_t = 65_536, value_parser = parse_budget)]
    budget_bytes: usize,
    #[arg(long)]
    cursor: Option<String>,
    #[command(flatten)]
    work: WorkArgs,
}
impl ReadArgs {
    fn options(&self) -> ReadOptions {
        ReadOptions {
            limit: 1,
            budget: self.budget_bytes,
            max_scan: 0,
            cursor: self.cursor.clone(),
        }
    }
}

impl Command {
    fn timeout_ms(&self) -> Option<u64> {
        match self {
            Self::Context { page, .. } => Some(page.work.timeout_ms),
            Self::Browse { page, .. } => Some(page.search.scan.work.timeout_ms),
            Self::Read { page, .. } => Some(page.work.timeout_ms),
            Self::Search { page, .. } => Some(page.scan.work.timeout_ms),
            Self::Neighbors { page, .. } => Some(page.scan.work.timeout_ms),
            Self::Validate { page, .. } => Some(page.work.timeout_ms),
            Self::Sync { page, .. } => Some(page.work.timeout_ms),
            Self::Index {
                command: IndexCommand::Verify { work, .. },
            } => Some(work.timeout_ms),
            Self::Bundle {
                command:
                    BundleCommand::Refresh { work, .. }
                    | BundleCommand::Add { work, .. }
                    | BundleCommand::Cache { work, .. },
            } => Some(work.timeout_ms),
            _ => None,
        }
    }
}

fn parse_range(value: &str, min: usize, max: usize) -> std::result::Result<usize, String> {
    value
        .parse::<usize>()
        .ok()
        .filter(|n| *n >= min && *n <= max)
        .ok_or_else(|| format!("expected an integer from {min} to {max}"))
}
fn parse_output(value: &str) -> std::result::Result<usize, String> {
    parse_range(value, 1024, 1_048_576)
}
fn parse_limit(value: &str) -> std::result::Result<usize, String> {
    parse_range(value, 1, 100)
}
fn parse_budget(value: &str) -> std::result::Result<usize, String> {
    parse_range(value, 1, 65_536)
}
fn parse_scan(value: &str) -> std::result::Result<usize, String> {
    parse_range(value, 1, 100_000)
}

// Clap can reject the request before Cli exists. Honor a valid output limit
// even on that path, taking the smallest limit if arguments are duplicated.
fn argument_output_limit(args: &[String]) -> usize {
    args.iter()
        .enumerate()
        .filter_map(|(index, arg)| {
            let value = if arg == "--max-output-bytes" {
                args.get(index + 1).map(String::as_str)
            } else {
                arg.strip_prefix("--max-output-bytes=")
            };
            value.and_then(|value| parse_output(value).ok())
        })
        .min()
        .unwrap_or(131072)
}

fn run(cli: &Cli, control: &RequestControl) -> Result<Response> {
    control.check()?;
    if matches!(cli.command, Command::BuildInfo) {
        return Ok(Response::new(
            "build-info",
            json!({
                "version": env!("CARGO_PKG_VERSION"),
                "target": env!("OKF_BUILD_TARGET"),
                "source_commit": match env!("OKF_BUILD_COMMIT") { "unknown" => None, value => Some(value) },
                "source_dirty": match env!("OKF_BUILD_DIRTY") { "true" => Some(true), "false" => Some(false), _ => None }
            }),
        ));
    }
    // Skill installation does not open the registry or database.
    if let Command::Skill {
        command: SkillCommand::Install { path },
    } = &cli.command
    {
        std::fs::create_dir_all(path).map_err(Error::io)?;
        let destination = path.join("SKILL.md");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .map_err(Error::io)?;
        file.write_all(include_bytes!("../../../skills/okf-cli/SKILL.md"))
            .map_err(Error::io)?;
        return Ok(Response::new("skill install", json!({"path": destination})));
    }
    let home = match &cli.home {
        Some(home) => home.clone(),
        None => default_home()?,
    };
    if !home.is_absolute() {
        return Err(Error::new(
            "INVALID_CONFIG",
            "--home must be an absolute path",
            2,
        ));
    }
    let registry = ConfigRegistry::discover(
        home.clone(),
        std::env::current_dir().map_err(Error::io)?,
        cli.scope.map(|s| s.label().into()),
    );
    if matches!(
        &cli.command,
        Command::Cache {
            command: CacheCommand::Reset
        }
    ) {
        return Ok(Response::new(
            "cache reset",
            SqliteCache::reset(&home.join("index.sqlite"))?,
        ));
    }
    let index_operation = matches!(
        &cli.command,
        Command::Sync { .. }
            | Command::Neighbors { .. }
            | Command::Index { .. }
            | Command::Read { indexed: true, .. }
            | Command::Search {
                full_text: true,
                ..
            }
            | Command::Context { content: true, .. }
    );
    let cache_result = if index_operation {
        None
    } else {
        Some(SqliteCache::open(&home.join("index.sqlite")))
    };
    let unavailable;
    let cache: &dyn Cache = match &cache_result {
        Some(Ok(cache)) => cache,
        Some(Err(error)) if cli.command.timeout_ms().is_some() => {
            unavailable = UnavailableCache(error.clone());
            &unavailable
        }
        Some(Err(error)) => return Err(error.clone()),
        None => {
            unavailable = UnavailableCache(Error::new(
                "CACHE_UNUSED",
                "index operations do not use the observation cache",
                5,
            ));
            &unavailable
        }
    };
    let reader = Readers;
    let app = App {
        registry: &registry,
        reader: &reader,
        cache,
        now: chrono::Utc::now(),
        control,
    };
    let mut response = match &cli.command {
        Command::Bundle { command } => match command {
            BundleCommand::Add {
                name,
                kind,
                path,
                url,
                reference,
                bundle_path,
                max_git_bytes,
                work: _,
            } => {
                if name.is_empty()
                    || name.len() > 64
                    || !name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                {
                    return Err(Error::new(
                        "INVALID_ALIAS",
                        "alias must be 1-64 ASCII letters, digits, - or _",
                        2,
                    ));
                }
                if kind != "fs" && kind != "git" {
                    return Err(Error::new(
                        "UNSUPPORTED_CAPABILITY",
                        format!(
                            "{kind} adapter is not implemented; this version supports fs and git"
                        ),
                        2,
                    ));
                }
                if kind == "fs"
                    && (url.is_some()
                        || reference.is_some()
                        || bundle_path.is_some()
                        || max_git_bytes.is_some())
                {
                    return Err(Error::new(
                        "INVALID_ARGUMENT",
                        "--url, --ref and --bundle-path require --kind git",
                        2,
                    ));
                }
                let root = if let Some(url) = url {
                    let key = sha256(
                        format!(
                            "{url}\0{name}\0{}\0{}",
                            registry.project.display(),
                            cli.scope.unwrap_or(Scope::Project).label()
                        )
                        .as_bytes(),
                    );
                    let root = home.join("git").join(key);
                    Git::init(&root, control)?;
                    root
                } else {
                    std::fs::canonicalize(path.as_ref().ok_or_else(|| {
                        Error::new("INVALID_ARGUMENT", "--path or --url is required", 2)
                    })?)
                    .map_err(Error::io)?
                };
                let scope = cli.scope.unwrap_or(Scope::Project).label();
                let key = format!(
                    "{name}\0{scope}\0{}\0{}",
                    root.display(),
                    registry.project.display()
                );
                let mut binding = BundleBinding {
                    id: format!("bundle-{}", &sha256(key.as_bytes())[..24]),
                    name: name.clone(),
                    kind: kind.clone(),
                    root: root.to_string_lossy().into_owned(),
                    scope: scope.into(),
                    git: None,
                };
                let _git_guard = if url.is_some() {
                    Some(Git::lock(&root)?)
                } else {
                    None
                };
                if kind == "git" {
                    let requested = reference.as_deref().unwrap_or("HEAD");
                    let fetched = url
                        .as_ref()
                        .map(|url| {
                            Git::fetch_bounded(
                                &root,
                                url,
                                requested,
                                control,
                                max_git_bytes.unwrap_or(268435456),
                            )
                        })
                        .transpose()?;
                    let mut revision = Git::pin(
                        &root,
                        fetched.as_deref().unwrap_or(requested),
                        bundle_path.as_deref().unwrap_or(""),
                        control,
                    )?;
                    revision.reference = requested.into();
                    revision.remote = url.clone();
                    binding.git = Some(revision);
                }
                reader.probe_controlled(&binding, control)?;
                registry.add(&binding)?;
                let mut response = Response::new("bundle add", json!({"bundle": binding}));
                response
                    .next
                    .push(json!({"operation": "browse", "target": format!("@{name}")}));
                Ok(response)
            }
            BundleCommand::List => Ok(Response::new(
                "bundle list",
                json!({"items": registry.list()?}),
            )),
            BundleCommand::Show { name } => Ok(Response::new(
                "bundle show",
                json!({"bundle": app.binding(name)?}),
            )),
            BundleCommand::Remove { name } => {
                let binding = app.binding(name)?;
                registry.remove(name, &binding.scope)?;
                Ok(Response::new(
                    "bundle remove",
                    json!({"removed": name, "bundle_content_deleted": false}),
                ))
            }
            BundleCommand::Cache { name, compact, .. } => {
                let binding = app.binding(name)?;
                if binding
                    .git
                    .as_ref()
                    .and_then(|g| g.remote.as_ref())
                    .is_none()
                {
                    return Err(Error::new(
                        "UNSUPPORTED_CAPABILITY",
                        "cache maintenance requires a managed remote binding",
                        2,
                    ));
                }
                let managed = std::fs::canonicalize(home.join("git")).map_err(Error::io)?;
                let root = std::fs::canonicalize(&binding.root).map_err(Error::io)?;
                if root.parent() != Some(managed.as_path()) {
                    return Err(Error::new(
                        "UNSUPPORTED_CAPABILITY",
                        "cache is outside the managed runtime directory",
                        2,
                    ));
                }
                Ok(Response::new(
                    "bundle cache",
                    Git::cache(&root, *compact, control)?,
                ))
            }
            BundleCommand::Refresh {
                name,
                max_git_bytes,
                ..
            } => {
                let old = app.binding(name)?;
                let git = old
                    .git
                    .as_ref()
                    .filter(|_| old.kind == "git")
                    .ok_or_else(|| {
                        Error::new(
                            "UNSUPPORTED_CAPABILITY",
                            "refresh requires a Git binding",
                            2,
                        )
                    })?;
                let root = std::path::Path::new(&old.root);
                let _git_guard = if git.remote.is_some() {
                    Some(Git::lock(root)?)
                } else {
                    None
                };
                let fetched = git
                    .remote
                    .as_ref()
                    .map(|url| {
                        Git::fetch_bounded(root, url, &git.reference, control, *max_git_bytes)
                    })
                    .transpose()?;
                let mut revision = Git::pin(
                    root,
                    fetched.as_deref().unwrap_or(&git.reference),
                    &git.bundle_path,
                    control,
                )?;
                revision.reference = git.reference.clone();
                revision.remote = git.remote.clone();
                let mut updated = old.clone();
                updated.git = Some(revision);
                registry.replace(&old, &updated)?;
                Ok(Response::new(
                    "bundle refresh",
                    json!({"bundle":updated,"previous_revision":git.commit}),
                ))
            }
        },
        Command::Context {
            bundles,
            query,
            page,
            content,
            filters,
        } => {
            if *content {
                return app.indexed_search_bundles_filtered(
                    query.as_deref().unwrap_or(""),
                    bundles,
                    &page.options(),
                    &SqliteIndex::open(&home.join("content.sqlite"))?,
                    true,
                    &filters.value(),
                );
            }
            if let Some(query) = query {
                let mut response =
                    app.search_filtered(query, bundles, &page.options(), &filters.value())?;
                response.command = "context".into();
                response.data["disclosure"] = json!("metadata_candidates");
                if let Some(items) = response.data["items"].as_array() {
                    for item in items {
                        response
                            .next
                            .push(json!({"operation": "read", "target": item["ref"]}));
                    }
                }
                Ok(response)
            } else {
                if !filters.is_empty() {
                    return Err(Error::new(
                        "INVALID_ARGUMENT",
                        "metadata filters require --query",
                        2,
                    ));
                }
                app.context(bundles, &page.options())
            }
        }
        Command::Browse { reference, page } => app.browse(reference, &page.options()),
        Command::Neighbors {
            reference,
            direction,
            page,
        } => app.neighbors(
            reference,
            direction,
            &page.options(),
            &SqliteIndex::open(&home.join("content.sqlite"))?,
        ),
        Command::Read {
            reference,
            raw,
            page,
            indexed,
            generation,
        } => {
            if *indexed {
                let response = app.indexed_read(
                    reference,
                    &page.options(),
                    &SqliteIndex::open(&home.join("content.sqlite"))?,
                )?;
                if generation.as_ref().is_some_and(|expected| {
                    response.data["document"]["generation"].as_str() != Some(expected)
                }) {
                    return Err(Error::new(
                        "REVISION_CHANGED",
                        "index generation changed; repeat search",
                        6,
                    ));
                }
                Ok(response)
            } else {
                app.read(reference, *raw, &page.options())
            }
        }
        Command::Search {
            query,
            bundles,
            page,
            full_text,
            filters,
        } => {
            if *full_text {
                app.indexed_search_bundles_filtered(
                    query,
                    bundles,
                    &page.options(),
                    &SqliteIndex::open(&home.join("content.sqlite"))?,
                    false,
                    &filters.value(),
                )
            } else {
                app.search_filtered(query, bundles, &page.options(), &filters.value())
            }
        }
        Command::Sync { name, page } => app.sync(
            name,
            &ReadOptions {
                limit: 0,
                budget: 0,
                max_scan: page.max_scan,
                cursor: None,
            },
            &SqliteIndex::open(&home.join("content.sqlite"))?,
        ),
        Command::Index { command } => {
            let path = home.join("content.sqlite");
            let index = match command {
                IndexCommand::Verify { .. } => SqliteIndex::open_read_only(&path)?,
                _ => SqliteIndex::open(&path)?,
            };
            match command {
                IndexCommand::Verify { name, max_scan, .. } => {
                    app.index_verify(name, *max_scan, &index)
                }
                IndexCommand::Status { name } => app.index_status(name, &index),
                IndexCommand::Clear { name } => {
                    index.clear(&app.binding(name)?.index_key())?;
                    Ok(Response::new("index clear", json!({"cleared":name})))
                }
            }
        }
        Command::Validate {
            name,
            page,
            max_resources,
            attachments,
            anchor_style,
            max_warnings: _,
        } => app.validate_with_checks(
            name,
            &ReadOptions {
                limit: 0,
                budget: 0,
                max_scan: page.max_scan,
                cursor: None,
            },
            *max_resources,
            *attachments,
            anchor_style.is_some(),
        ),
        Command::Status { name } => {
            let bindings = if let Some(name) = name {
                vec![app.binding(name)?]
            } else {
                registry.list()?
            };
            let mut items = vec![];
            for binding in bindings {
                items
                    .push(json!({"bundle": binding, "cache": cache.status(&binding.cache_key())?}));
            }
            Ok(Response::new("status", json!({"items": items})))
        }
        Command::Doctor => Ok(Response::new(
            "doctor",
            json!({"version": env!("CARGO_PKG_VERSION"),
            "home": home, "project": registry.project, "bundles": registry.list()?.len(),
            "adapters": ["fs", "git"], "sqlite": "ready", "entrypoint": "cli"}),
        )),
        Command::Skill { .. } | Command::Cache { .. } | Command::BuildInfo => unreachable!(),
    }?;
    if let Some(Err(error)) = &cache_result {
        if !response
            .diagnostics
            .iter()
            .any(|d| d.code == "CACHE_WRITE_FAILED")
        {
            if response.status == "error" {
                response.diagnostics.push(Diagnostic::new(
                    "CACHE_UNAVAILABLE",
                    &error.message,
                    None,
                ));
            } else {
                response.partial("CACHE_UNAVAILABLE", &error.message);
            }
        }
    }
    if let Command::Validate {
        max_warnings: Some(limit),
        ..
    } = &cli.command
    {
        response.apply_warning_limit(*limit);
    }
    Ok(response)
}

fn error_response(command: &str, error: Error) -> Response {
    let mut response = Response::new(
        command,
        json!({"error": {"code": error.code, "message": error.message}}),
    );
    response.status = "error".into();
    response.exit_code = error.exit_code;
    response
}

fn emit(mut response: Response, format: Format, limit: usize) -> ExitCode {
    response.request_id = format!(
        "req-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_micros()
    );
    let mut output =
        serde_json::to_string(&response).unwrap_or_else(|_| "{\"status\":\"error\"}".into());
    if output.len() + 1 > limit {
        response = error_response(
            &response.command,
            Error::new(
                "OUTPUT_TOO_LARGE",
                "response exceeds --max-output-bytes; reduce the page or content budget",
                7,
            ),
        );
        response.request_id = format!(
            "req-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_micros()
        );
        output = serde_json::to_string(&response).unwrap_or_default();
    }
    if matches!(format, Format::Text) {
        output = serde_json::to_string_pretty(&response).unwrap_or(output);
        if output.len() + 1 > limit {
            output = serde_json::to_string(&response).unwrap_or_default();
        }
    }
    output.push('\n');
    match std::io::stdout().lock().write_all(output.as_bytes()) {
        Ok(()) => ExitCode::from(response.exit_code),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(_) => ExitCode::from(5),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let json_requested = args
        .windows(2)
        .any(|p| p[0] == "--format" && p[1] == "json")
        || args.iter().any(|s| s == "--format=json");
    let cli = match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(error) if error.use_stderr() && json_requested => {
            let limit = argument_output_limit(&args);
            let mut message = error.to_string();
            // Leave room for the JSON envelope and keep malformed requests at
            // exit code 2 instead of turning them into output-limit errors.
            let mut truncated = false;
            while serde_json::to_string(&message).unwrap_or_default().len() > limit / 2 {
                let mut end = message.len() / 2;
                while !message.is_char_boundary(end) {
                    end -= 1;
                }
                message.truncate(end);
                truncated = true;
            }
            if truncated {
                message.push_str(" [truncated]");
            }
            return emit(
                error_response("arguments", Error::new("INVALID_ARGUMENT", message, 2)),
                Format::Json,
                limit,
            );
        }
        Err(error) => {
            let code = error.exit_code() as u8;
            let _ = error.print();
            return ExitCode::from(code);
        }
    };
    let cancelled = Arc::new(AtomicBool::new(false));
    if cli.command.timeout_ms().is_some() {
        let signal = Arc::clone(&cancelled);
        if let Err(error) = ctrlc::set_handler(move || signal.store(true, Ordering::Relaxed)) {
            return emit(
                error_response("request", Error::io(error)),
                cli.format,
                cli.max_output_bytes,
            );
        }
    }
    let control = RequestControl::new(
        Duration::from_millis(cli.command.timeout_ms().unwrap_or(30_000)),
        cancelled,
    );
    let response = run(&cli, &control).unwrap_or_else(|error| error_response("request", error));
    emit(response, cli.format, cli.max_output_bytes)
}
