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
    /// Operate on a single ticket's claim (recovery, not day-to-day work).
    Ticket {
        #[command(subcommand)]
        command: TicketCommand,
    },
    /// Populate a database with demo content, for a local instance you can
    /// actually look at (see the `backlot.yml` datastore presets).
    Seed {
        /// `dev` = a demo project with tickets across every workflow state and
        /// questions of every kind; `empty` = schema only, no content.
        #[arg(long, default_value = "dev")]
        preset: String,
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
        /// Print one JSON object (including the plaintext `token`) instead of
        /// the human-readable block — for scripts and provisioning hooks.
        #[arg(long)]
        json: bool,
    },
    /// List tokens (never shows plaintext).
    List,
    /// Revoke a token by its id (see `token list`).
    Revoke { id: String },
}

#[derive(Subcommand)]
enum TicketCommand {
    /// Drop a ticket's claim whatever its state, bumping the fence so the
    /// displaced worker's next write is refused.
    ///
    /// Here as well as over HTTP (`POST /v1/tickets/{id}/force-release`,
    /// admin scope) because this is the command you need when the reason the
    /// ticket is stuck is that the server is wedged, or when the admin token
    /// itself is what got lost — shell access is the root of trust.
    ForceRelease {
        /// Ticket id.
        id: String,
        /// Why it was forced; recorded on the `lease_revoked` event.
        #[arg(long)]
        reason: Option<String>,
        /// Actor recorded as having forced it.
        #[arg(long, default_value = "cli:admin")]
        actor: String,
    },
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
        /// House style for the text agents write on this project — ticket
        /// titles/bodies, comments, and questions. Omit for no preference.
        #[arg(long)]
        style: Option<String>,
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
    /// Set (or clear) a project's style guide for agent-written text.
    Style {
        /// Project id.
        id: String,
        /// The style guide. Omit with --clear (or --file) to remove/replace it.
        style: Option<String>,
        /// Read the style guide from a file instead of the argument.
        #[arg(long)]
        file: Option<String>,
        /// Clear the style guide instead of setting it.
        #[arg(long)]
        clear: bool,
    },
    /// Set (or clear) how long an answer link for this project's questions lives.
    AnswerTtl {
        /// Project id.
        id: String,
        /// The lifetime: 7d, 24h, 90m, 3600s, or a plain second count (max 30d).
        /// Omit to print the current setting; pass --clear to fall back to the
        /// built-in default.
        ttl: Option<String>,
        /// Clear the project default instead of setting it.
        #[arg(long)]
        clear: bool,
    },
    /// Show or set this project's lease policy: the lease a claim gets by
    /// default, and the ceiling an explicit ttl_seconds is checked against.
    ///
    /// With no flags, prints the values in force. The two are stored (and
    /// validated) as a pair, so a flag you omit keeps its stored value rather
    /// than being cleared.
    ClaimTtl {
        /// Project id.
        id: String,
        /// Lease for a claim that names no ttl_seconds: 30m, 1h, 900s, or a
        /// plain second count. Omit to keep the stored value.
        #[arg(long, value_name = "TTL")]
        default: Option<String>,
        /// Ceiling an explicit ttl_seconds is checked against — same forms, no
        /// upper bound. Omit to keep the stored value.
        #[arg(long, value_name = "TTL")]
        max: Option<String>,
        /// Clear the project's default lease, falling back to the built-in.
        #[arg(long)]
        clear_default: bool,
        /// Clear the project's ceiling, falling back to the built-in.
        #[arg(long)]
        clear_max: bool,
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
        Command::Ticket { command } => run_ticket(&cli.db, command),
        Command::Seed { preset } => run_seed(&cli.db, &preset),
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
            json,
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
            if json {
                // The plaintext rides in this object — same "shown once" rule as
                // the human block; a caller that discards it cannot recover it.
                let out = serde_json::json!({
                    "id": row.id,
                    "actor": row.actor,
                    "scopes": row.scopes,
                    "projects": row.projects,
                    "expires_at": row.expires_at.map(iso),
                    "rate_limit": row.rate_limit,
                    "token": plaintext,
                });
                println!("{out}");
                return Ok(());
            }
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

fn run_ticket(db: &str, command: TicketCommand) -> Result<(), String> {
    let store = open_store(db)?;
    match command {
        TicketCommand::ForceRelease { id, reason, actor } => {
            let forced = store
                .force_release(&id, &actor, reason.as_deref())
                .map_err(|e| e.into_message())?;
            println!(
                "force-released {} from '{}'{}; fence {} -> {} (the old holder's next write now 409s)",
                forced.ticket,
                forced.previous_holder,
                if forced.lease_expired {
                    " (its lease had already expired)"
                } else {
                    ""
                },
                forced.previous_fence,
                forced.fence,
            );
            Ok(())
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
            style,
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
                        serde_norway::from_str(&raw)
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
            if let Some(style) = style {
                project = store
                    .set_style_guide(&id, Some(&style), "cli:admin")
                    .map_err(|e| e.into_message())?;
            }
            println!(
                "created project '{}' ({}) with workflow '{}'{}{}",
                project.id,
                project.name,
                project.workflow.name,
                project
                    .question_language
                    .map(|l| format!("; question language: {l}"))
                    .unwrap_or_default(),
                project
                    .style_guide
                    .map(|_| "; style guide set".to_string())
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
        ProjectCommand::Style {
            id,
            style,
            file,
            clear,
        } => {
            // --file reads the guide from disk ("-" = stdin), so a multi-line
            // house style doesn't have to survive shell quoting.
            let from_file = match &file {
                None => None,
                Some(path) if path == "-" => {
                    let mut buf = String::new();
                    std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)
                        .map_err(|e| format!("cannot read the style guide from stdin: {e}"))?;
                    Some(buf)
                }
                Some(path) => Some(
                    std::fs::read_to_string(path)
                        .map_err(|e| format!("cannot read style file '{path}': {e}"))?,
                ),
            };
            if clear && (style.is_some() || from_file.is_some()) {
                return Err("--clear takes no value".to_string());
            }
            let text = from_file.or(style);
            if text.is_none() && !clear {
                return Err(
                    "provide a style guide (or --file PATH), or --clear to remove it".to_string(),
                );
            }
            let project = store
                .set_style_guide(&id, text.as_deref(), "cli:admin")
                .map_err(|e| e.into_message())?;
            match project.style_guide {
                Some(s) => println!("project '{}' style guide set to:\n{s}", project.id),
                None => println!("project '{}' style guide cleared", project.id),
            }
            Ok(())
        }
        ProjectCommand::AnswerTtl { id, ttl, clear } => {
            if clear && ttl.is_some() {
                return Err("--clear takes no value".to_string());
            }
            if ttl.is_none() && !clear {
                // Reading is not a write, so it does not need --clear to be safe.
                let project = store
                    .get_project(&id)
                    .map_err(|e| e.into_message())?
                    .ok_or_else(|| format!("no project '{id}'"))?;
                match project.answer_link_ttl_seconds {
                    Some(s) => println!("project '{id}' answer-link lifetime: {s}s"),
                    None => println!(
                        "project '{id}' sets no answer-link lifetime (built-in default: {}s)",
                        takomo::store::DEFAULT_ANSWER_TTL_SECONDS
                    ),
                }
                return Ok(());
            }
            let secs = match ttl {
                None => None,
                Some(raw) => Some(parse_ttl_seconds(&raw)?),
            };
            let project = store
                .set_answer_link_ttl(&id, secs, "cli:admin")
                .map_err(|e| e.into_message())?;
            match project.answer_link_ttl_seconds {
                Some(s) => println!("project '{}' answer-link lifetime set to {s}s", project.id),
                None => println!(
                    "project '{}' answer-link lifetime cleared (built-in default: {}s)",
                    project.id,
                    takomo::store::DEFAULT_ANSWER_TTL_SECONDS
                ),
            }
            Ok(())
        }
        ProjectCommand::ClaimTtl {
            id,
            default,
            max,
            clear_default,
            clear_max,
        } => {
            // Reading is not a write, so it needs no confirming flag.
            let project = store
                .get_project(&id)
                .map_err(|e| e.into_message())?
                .ok_or_else(|| format!("no project '{id}'"))?;
            if default.is_none() && max.is_none() && !clear_default && !clear_max {
                print_claim_ttls(&id, "lease policy", &project);
                return Ok(());
            }
            let (ttl, max_ttl) = resolve_claim_ttls(
                default.as_deref(),
                clear_default,
                max.as_deref(),
                clear_max,
                (project.claim_ttl_seconds, project.max_claim_ttl_seconds),
            )?;
            // The store's message is the shared contract text and names the JSON
            // fields (`claim_ttl_seconds`, "send null"). Translate that to the
            // flags the reader actually typed rather than reimplementing the
            // validation here.
            let project = store
                .set_claim_ttls(&id, ttl, max_ttl, "cli:admin")
                .map_err(|e| {
                    format!(
                        "{}\nhint: on the CLI claim_ttl_seconds is --default and \
                         max_claim_ttl_seconds is --max; where the API takes null, pass \
                         --clear-default / --clear-max.",
                        e.into_message()
                    )
                })?;
            print_claim_ttls(&id, "lease policy now", &project);
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

/// Parse a lifetime — `7d`, `24h`, `90m`, `3600s`, or a plain second count —
/// into seconds. The same forms `takomo share --ttl` and `takomo answer-link
/// --ttl` accept, so a duration reads the same wherever it is typed.
fn parse_ttl_seconds(raw: &str) -> Result<i64, String> {
    let raw = raw.trim();
    let bad = || format!("invalid ttl '{raw}': use 7d, 24h, 90m, 3600s, or a plain second count");
    let (num, mult) = match raw.strip_suffix(['d', 'h', 'm', 's']) {
        Some(n) => (
            n,
            match raw.chars().last() {
                Some('d') => 86_400,
                Some('h') => 3_600,
                Some('m') => 60,
                _ => 1,
            },
        ),
        None => (raw, 1),
    };
    let n: i64 = num.parse().map_err(|_| bad())?;
    n.checked_mul(mult).ok_or_else(bad)
}

/// Resolve what one `project claim-ttl` write should store, from the flags plus
/// the `(default, max)` pair already in the row.
///
/// The store writes both columns in one UPDATE (they are validated as a pair by
/// `normalize_claim_ttls`), so an **omitted flag must re-send the stored value**
/// — otherwise setting one half would silently clear the other. Same rule as
/// `PUT /v1/projects/{project}/claim-ttl`, where an absent field means "leave as
/// it is" and an explicit null clears; here `--clear-default` / `--clear-max`
/// are that null.
///
/// Kept separate from the command arm so this rule is unit-testable.
fn resolve_claim_ttls(
    default: Option<&str>,
    clear_default: bool,
    max: Option<&str>,
    clear_max: bool,
    stored: (Option<i64>, Option<i64>),
) -> Result<(Option<i64>, Option<i64>), String> {
    let resolve = |raw: Option<&str>,
                   clear: bool,
                   stored: Option<i64>,
                   set_flag: &str,
                   clear_flag: &str,
                   built_in: i64|
     -> Result<Option<i64>, String> {
        if clear {
            if raw.is_some() {
                return Err(format!(
                    "{set_flag} and {clear_flag} conflict: pass {set_flag} <ttl> to set this \
                     value, or {clear_flag} alone to drop the project setting and fall back to \
                     the built-in {built_in}s."
                ));
            }
            return Ok(None);
        }
        match raw {
            // Absent means "leave as it is": this writes both columns at once,
            // so the stored value has to be re-sent, not dropped.
            None => Ok(stored),
            Some(raw) => Ok(Some(parse_ttl_seconds(raw)?)),
        }
    };
    let ttl = resolve(
        default,
        clear_default,
        stored.0,
        "--default",
        "--clear-default",
        takomo::store::DEFAULT_TTL_SECONDS,
    )?;
    let max_ttl = resolve(
        max,
        clear_max,
        stored.1,
        "--max",
        "--clear-max",
        takomo::store::MAX_TTL_SECONDS,
    )?;
    Ok((ttl, max_ttl))
}

/// Print a project's lease policy as the numbers actually **in force** — an
/// operator wants the seconds a claim will get, not "unset", so an unset column
/// prints the built-in it falls back to and says so.
fn print_claim_ttls(id: &str, label: &str, project: &takomo::store::Project) {
    let show = |v: Option<i64>, built_in: i64| match v {
        Some(s) => format!("{s}s (project setting)"),
        None => format!("{built_in}s (built-in default; this project sets none)"),
    };
    println!("project '{id}' {label}:");
    println!(
        "  default lease: {} — what a claim that names no ttl_seconds gets",
        show(
            project.claim_ttl_seconds,
            takomo::store::DEFAULT_TTL_SECONDS
        )
    );
    println!(
        "  maximum lease: {} — the ceiling an explicit ttl_seconds is checked against",
        show(
            project.max_claim_ttl_seconds,
            takomo::store::MAX_TTL_SECONDS
        )
    );
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

// ---------------------------------------------------------------------------
// seed — demo content for a local instance (backlot.yml datastore presets)
// ---------------------------------------------------------------------------
//
// The content itself lives in `takomo::seed`, which drives the real state
// machine and is tested there; this is just the CLI shell around it.

fn run_seed(db: &str, preset: &str) -> Result<(), String> {
    // Opening the store runs the migrations — which is the whole of `empty`.
    let store = open_store(db)?;
    match preset {
        "empty" => {
            println!("preset 'empty': schema only, no demo content.");
            Ok(())
        }
        "dev" => {
            let s = takomo::seed::dev(&store).map_err(|e| e.into_message())?;
            if s.skipped {
                println!("project '{}' already exists; nothing to seed.", s.project);
            } else {
                println!(
                    "seeded project '{}': {} tickets across every workflow state, {} questions.",
                    s.project, s.tickets, s.questions
                );
            }
            Ok(())
        }
        other => Err(format!(
            "unknown preset '{other}'. Use 'dev' (demo content) or 'empty' (schema only)."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_claim_ttls;

    /// The trap this command has to avoid: `set_claim_ttls` writes both columns
    /// in one UPDATE, so setting or clearing one half must re-send the other
    /// half's stored value instead of nulling it.
    #[test]
    fn omitted_flag_keeps_the_stored_value() {
        let stored = (Some(1800), Some(7200));
        assert_eq!(
            resolve_claim_ttls(Some("30m"), false, None, false, stored),
            Ok((Some(1800), Some(7200)))
        );
        assert_eq!(
            resolve_claim_ttls(None, false, Some("4h"), false, stored),
            Ok((Some(1800), Some(14_400)))
        );
        // Clearing one half leaves the other exactly as it was.
        assert_eq!(
            resolve_claim_ttls(None, true, None, false, stored),
            Ok((None, Some(7200)))
        );
        assert_eq!(
            resolve_claim_ttls(None, false, None, true, stored),
            Ok((Some(1800), None))
        );
    }

    #[test]
    fn both_flags_are_parsed_as_durations() {
        assert_eq!(
            resolve_claim_ttls(Some("20m"), false, Some("2h"), false, (None, None)),
            Ok((Some(1200), Some(7200)))
        );
        assert_eq!(
            resolve_claim_ttls(Some("900"), false, None, true, (None, Some(7200))),
            Ok((Some(900), None))
        );
    }

    #[test]
    fn a_bad_duration_names_the_accepted_forms() {
        let err = resolve_claim_ttls(Some("half an hour"), false, None, false, (None, None))
            .expect_err("a non-duration must be refused");
        assert!(err.contains("invalid ttl"), "{err}");
        assert!(err.contains("3600s"), "{err}");
    }

    #[test]
    fn setting_and_clearing_the_same_half_is_refused() {
        let err = resolve_claim_ttls(Some("30m"), true, None, false, (None, None))
            .expect_err("--default with --clear-default is ambiguous");
        assert!(err.contains("--default"), "{err}");
        assert!(err.contains("--clear-default"), "{err}");
        let err = resolve_claim_ttls(None, false, Some("2h"), true, (None, None))
            .expect_err("--max with --clear-max is ambiguous");
        assert!(err.contains("--clear-max"), "{err}");
    }
}
