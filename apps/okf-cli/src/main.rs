use clap::{Args, Parser, Subcommand, ValueEnum};
use okf_adapters::{default_home, ConfigRegistry, Filesystem, SqliteCache};
use okf_application::{
    App, BundleBinding, BundleReader, BundleRegistry, Cache, ReadOptions, Response,
};
use okf_core::{sha256, Error, Result};
use serde_json::json;
use std::{io::Write, path::PathBuf, process::ExitCode};

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
        #[command(flatten)]
        page: PageArgs,
    },
    /// Expand one directory. Reserved index text is available through read.
    Browse {
        reference: String,
        #[command(flatten)]
        page: PageArgs,
    },
    /// Read exact original UTF-8 bytes, with revision-checked continuation.
    Read {
        reference: String,
        #[arg(long)]
        raw: bool,
        #[command(flatten)]
        page: PageArgs,
    },
    /// Bounded metadata search; this is not full-text search.
    Search {
        query: String,
        #[arg(long = "bundle")]
        bundles: Vec<String>,
        #[command(flatten)]
        page: PageArgs,
    },
    Status {
        name: Option<String>,
    },
    Validate {
        name: String,
        #[command(flatten)]
        page: PageArgs,
    },
    Doctor,
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
    },
    List,
    Show {
        name: String,
    },
    Remove {
        name: String,
    },
}

#[derive(Subcommand)]
enum SkillCommand {
    /// Install the bundled CLI consumption skill. Existing files are preserved.
    Install { path: PathBuf },
}

#[derive(Args)]
struct PageArgs {
    #[arg(long, default_value_t = 20, value_parser = parse_limit)]
    limit: usize,
    #[arg(long, value_parser = parse_budget)]
    budget_bytes: Option<usize>,
    #[arg(long, default_value_t = 100, value_parser = parse_scan)]
    max_scan: usize,
    #[arg(long)]
    cursor: Option<String>,
}
impl PageArgs {
    fn options(&self, default_budget: usize) -> ReadOptions {
        ReadOptions {
            limit: self.limit,
            budget: self.budget_bytes.unwrap_or(default_budget),
            max_scan: self.max_scan,
            cursor: self.cursor.clone(),
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
    parse_range(value, 1, 10_000)
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

fn run(cli: &Cli) -> Result<Response> {
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
        file.write_all(include_bytes!("../../../skills/okf-consume/SKILL.md"))
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
    let cache = SqliteCache::open(&home.join("index.sqlite"))?;
    let reader = Filesystem;
    let app = App {
        registry: &registry,
        reader: &reader,
        cache: &cache,
        now: chrono::Utc::now(),
    };
    match &cli.command {
        Command::Bundle { command } => match command {
            BundleCommand::Add { name, kind, path } => {
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
                if kind != "fs" {
                    return Err(Error::new(
                        "UNSUPPORTED_CAPABILITY",
                        format!("{kind} adapter is not implemented; this version supports fs"),
                        2,
                    ));
                }
                let path = path.as_ref().ok_or_else(|| {
                    Error::new("INVALID_ARGUMENT", "--path is required for fs", 2)
                })?;
                let root = std::fs::canonicalize(path).map_err(Error::io)?;
                let scope = cli.scope.unwrap_or(Scope::Project).label();
                let key = format!(
                    "{name}\0{scope}\0{}\0{}",
                    root.display(),
                    registry.project.display()
                );
                let binding = BundleBinding {
                    id: format!("bundle-{}", &sha256(key.as_bytes())[..24]),
                    name: name.clone(),
                    kind: kind.clone(),
                    root: root.to_string_lossy().into_owned(),
                    scope: scope.into(),
                };
                reader.probe(&binding)?;
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
        },
        Command::Context {
            bundles,
            query,
            page,
        } => {
            if let Some(query) = query {
                let mut response = app.search(query, bundles, &page.options(24 * 1024))?;
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
                app.context(bundles, &page.options(24 * 1024))
            }
        }
        Command::Browse { reference, page } => app.browse(reference, &page.options(16 * 1024)),
        Command::Read {
            reference,
            raw,
            page,
        } => app.read(reference, *raw, &page.options(64 * 1024)),
        Command::Search {
            query,
            bundles,
            page,
        } => app.search(query, bundles, &page.options(16 * 1024)),
        Command::Validate { name, page } => app.validate(name, &page.options(16 * 1024)),
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
            "adapters": ["fs"], "sqlite": "ready", "entrypoint": "cli"}),
        )),
        Command::Skill { .. } => unreachable!(),
    }
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
    let response = run(&cli).unwrap_or_else(|error| error_response("request", error));
    emit(response, cli.format, cli.max_output_bytes)
}
