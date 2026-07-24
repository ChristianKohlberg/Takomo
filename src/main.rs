//! takomo — single binary: server plus `token` and `project` admin
//! subcommands (which operate on the database directly; shell access to the
//! server is the root of trust, see spec/auth.md).

use clap::{Parser, Subcommand};
use takomo::ids::{iso, now_ms};
use takomo::store::Store;
use takomo::workflow::Workflow;

#[derive(Parser)]
#[command(
    name = "takomo",
    version,
    about = "Central task store for AI agent fleets"
)]
struct Cli {
    /// Path to the SQLite database file.
    #[arg(long, global = true, env = "TAKOMO_DB", default_value = "takomo.db")]
    db: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the HTTP server.
    Serve {
        /// Bind address. Non-loopback requires TAKOMO_ALLOW_PUBLIC_BIND=1.
        #[arg(long, env = "TAKOMO_BIND", default_value = "127.0.0.1:8080")]
        bind: String,
        /// Expired-lease sweep interval in seconds.
        #[arg(long, env = "TAKOMO_SWEEP_SECONDS", default_value_t = 10)]
        sweep_seconds: u64,
    },
    /// Manage bearer tokens (mint, list, revoke).
    Token {
        #[command(subcommand)]
        command: TokenCommand,
    },
    /// Manage projects.
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
}

#[derive(Subcommand)]
enum TokenCommand {
    /// Mint a token; the plaintext is printed once and stored only as a hash.
    Create {
        /// Actor name, e.g. human:alice, orch:main, agent:runner-1.
        #[arg(long)]
        actor: String,
        /// Comma-separated scopes: read,write,human,autoland,admin, or free-form.
        #[arg(long, default_value = "read,write")]
        scopes: String,
        /// Comma-separated project ids, or '*' for all.
        #[arg(long, default_value = "*")]
        projects: String,
        /// Expiry like 90d, 12h, 30m, or an RFC 3339 timestamp.
        #[arg(long)]
        expires: Option<String>,
        /// Write budget per minute (sliding window).
        #[arg(long, default_value_t = 120)]
        rate_limit: i64,
    },
    /// List tokens (never shows plaintext).
    List,
    /// Revoke a token by its id (see `token list`).
    Revoke { id: String },
}

#[derive(Subcommand)]
enum ProjectCommand {
    /// Create a project (default workflow: built-in factory-default).
    Create {
        /// Short slug, ^[a-z][a-z0-9-]{1,15}$; becomes the ticket id prefix.
        #[arg(long)]
        id: String,
        #[arg(long)]
        name: String,
        /// Path to a workflow definition (YAML or JSON); omit for factory-default.
        #[arg(long)]
        workflow: Option<String>,
        /// Human-facing language agents should phrase ask-a-human questions in
        /// (e.g. "German"). Omit for no preference.
        #[arg(long)]
        language: Option<String>,
    },
    /// Set (or clear) a project's ask-a-human question language.
    Language {
        /// Project id.
        id: String,
        /// The language, e.g. "German". Omit with --clear to remove it.
        language: Option<String>,
        /// Clear the language instead of setting it.
        #[arg(long)]
        clear: bool,
    },
    /// List projects.
    List,
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Serve {
            bind,
            sweep_seconds,
        } => tokio::runtime::Runtime::new()
            .expect("tokio runtime")
            .block_on(takomo::server::serve(&bind, &cli.db, sweep_seconds)),
        Command::Token { command } => run_token(&cli.db, command),
        Command::Project { command } => run_project(&cli.db, command),
    };
    if let Err(msg) = result {
        eprintln!("error: {msg}");
        std::process::exit(1);
    }
}

fn open_store(db: &str) -> Result<Store, String> {
    Store::open(db).map_err(|e| e.into_message())
}

fn run_token(db: &str, command: TokenCommand) -> Result<(), String> {
    let store = open_store(db)?;
    match command {
        TokenCommand::Create {
            actor,
            scopes,
            projects,
            expires,
            rate_limit,
        } => {
            let scopes: Vec<String> = scopes
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            let projects_opt: Option<Vec<String>> = if projects.trim() == "*" {
                None
            } else {
                Some(
                    projects
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect(),
                )
            };
            let expires_at = expires.as_deref().map(parse_expiry).transpose()?;
            let (row, plaintext) = store
                .create_token(
                    &actor,
                    &scopes,
                    projects_opt.as_deref(),
                    rate_limit,
                    expires_at,
                )
                .map_err(|e| e.into_message())?;
            println!("token id:  {}", row.id);
            println!("actor:     {}", row.actor);
            println!("scopes:    {}", row.scopes.join(","));
            println!(
                "projects:  {}",
                row.projects
                    .map(|p| p.join(","))
                    .unwrap_or_else(|| "*".into())
            );
            println!(
                "expires:   {}",
                row.expires_at.map(iso).unwrap_or_else(|| "never".into())
            );
            println!("rate:      {}/min writes", row.rate_limit);
            println!();
            println!("{plaintext}");
            println!();
            println!("This plaintext is shown ONCE; only its SHA-256 is stored.");
            Ok(())
        }
        TokenCommand::List => {
            let tokens = store.list_tokens().map_err(|e| e.into_message())?;
            if tokens.is_empty() {
                println!("no tokens; mint one with: takomo token create --actor <name>");
                return Ok(());
            }
            println!(
                "{:<14} {:<24} {:<24} {:<12} {:<22} {:<10} LAST USED",
                "ID", "ACTOR", "SCOPES", "PROJECTS", "EXPIRES", "REVOKED"
            );
            for t in tokens {
                println!(
                    "{:<14} {:<24} {:<24} {:<12} {:<22} {:<10} {}",
                    t.id,
                    t.actor,
                    t.scopes.join(","),
                    t.projects
                        .map(|p| p.join(","))
                        .unwrap_or_else(|| "*".into()),
                    t.expires_at.map(iso).unwrap_or_else(|| "never".into()),
                    if t.revoked_at.is_some() { "yes" } else { "no" },
                    t.last_used_at.map(iso).unwrap_or_else(|| "never".into()),
                );
            }
            Ok(())
        }
        TokenCommand::Revoke { id } => {
            let revoked = store.revoke_token(&id).map_err(|e| e.into_message())?;
            if revoked {
                println!("revoked {id}");
                Ok(())
            } else {
                Err(format!(
                    "no active token with id '{id}' (see: takomo token list)"
                ))
            }
        }
    }
}

fn run_project(db: &str, command: ProjectCommand) -> Result<(), String> {
    let store = open_store(db)?;
    match command {
        ProjectCommand::Create {
            id,
            name,
            workflow,
            language,
        } => {
            let wf: Option<Workflow> = match workflow {
                None => None,
                Some(path) => {
                    let raw = std::fs::read_to_string(&path)
                        .map_err(|e| format!("cannot read workflow file '{path}': {e}"))?;
                    let wf = if path.ends_with(".json") {
                        serde_json::from_str(&raw)
                            .map_err(|e| format!("invalid workflow JSON: {e}"))?
                    } else {
                        serde_yaml::from_str(&raw)
                            .map_err(|e| format!("invalid workflow YAML: {e}"))?
                    };
                    Some(wf)
                }
            };
            let mut project = store
                .create_project(&id, &name, wf, "cli:admin")
                .map_err(|e| e.into_message())?;
            if let Some(lang) = language {
                project = store
                    .set_question_language(&id, Some(&lang), "cli:admin")
                    .map_err(|e| e.into_message())?;
            }
            println!(
                "created project '{}' ({}) with workflow '{}'{}",
                project.id,
                project.name,
                project.workflow.name,
                project
                    .question_language
                    .map(|l| format!("; question language: {l}"))
                    .unwrap_or_default()
            );
            Ok(())
        }
        ProjectCommand::Language {
            id,
            language,
            clear,
        } => {
            let lang: Option<&str> = if clear { None } else { language.as_deref() };
            if lang.is_none() && !clear {
                return Err("provide a language, or --clear to remove it".to_string());
            }
            let project = store
                .set_question_language(&id, lang, "cli:admin")
                .map_err(|e| e.into_message())?;
            match project.question_language {
                Some(l) => println!("project '{}' question language set to: {l}", project.id),
                None => println!("project '{}' question language cleared", project.id),
            }
            Ok(())
        }
        ProjectCommand::List => {
            let projects = store.list_projects().map_err(|e| e.into_message())?;
            if projects.is_empty() {
                println!(
                    "no projects; create one with: takomo project create --id <slug> --name <name>"
                );
                return Ok(());
            }
            println!("{:<18} {:<32} {:<20} CREATED", "ID", "NAME", "WORKFLOW");
            for p in projects {
                println!(
                    "{:<18} {:<32} {:<20} {}",
                    p.id,
                    p.name,
                    p.workflow.name,
                    iso(p.created_at)
                );
            }
            Ok(())
        }
    }
}

/// Parse `90d`, `12h`, `30m`, or an RFC 3339 timestamp into unix ms.
fn parse_expiry(raw: &str) -> Result<i64, String> {
    let raw = raw.trim();
    if let Some(num) = raw.strip_suffix('d') {
        let days: i64 = num.parse().map_err(|_| format!("invalid expiry '{raw}'"))?;
        return Ok(now_ms() + days * 86_400_000);
    }
    if let Some(num) = raw.strip_suffix('h') {
        let hours: i64 = num.parse().map_err(|_| format!("invalid expiry '{raw}'"))?;
        return Ok(now_ms() + hours * 3_600_000);
    }
    if let Some(num) = raw.strip_suffix('m') {
        let mins: i64 = num.parse().map_err(|_| format!("invalid expiry '{raw}'"))?;
        return Ok(now_ms() + mins * 60_000);
    }
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.timestamp_millis())
        .map_err(|_| format!("invalid expiry '{raw}': use 90d, 12h, 30m, or RFC 3339"))
}
