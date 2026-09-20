use anyhow::{Result, bail};
use clap::{Args, Parser, Subcommand};
use mnemosyne::{api, schema::serialize_memory, store::*};
use serde_json::{Value, json};
use std::{io, path::PathBuf};

#[derive(Parser)]
#[command(version, about = "Local-first, agent-agnostic memory kernel")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Args)]
struct Scope {
    #[arg(long,default_value="all",value_parser=["all","global","project"])]
    scope: String,
}
#[derive(Args)]
struct InstallOptions {
    #[arg(long)]
    dry_run: bool,
}
#[derive(Subcommand)]
enum Command {
    Proposal {
        #[arg(value_parser=["create","show","approve","reject","undo"])]
        action: String,
        #[arg(long, default_value = "")]
        id: String,
        #[arg(long, default_value = "")]
        confirm_hash: String,
        #[arg(long,default_value="project",value_parser=["global","project"])]
        scope: String,
    },
    Reconcile {
        #[arg(long)]
        commit: bool,
        #[arg(long,default_value="project",value_parser=["global","project"])]
        scope: String,
    },
    Sleep {
        #[arg(value_parser=["rules","export","import","cursor"])]
        action: String,
        #[arg(long, default_value_t = 0)]
        cursor: usize,
        #[arg(long)]
        snapshot: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long,default_value="project",value_parser=["global","project"])]
        scope: String,
    },
    View {
        #[arg(value_parser=["generate","inspect"])]
        action: String,
        #[arg(long, default_value = "architecture")]
        name: String,
        #[arg(long,default_value="project",value_parser=["global","project"])]
        scope: String,
    },
    Snapshot {
        destination: PathBuf,
        #[arg(long,default_value="project",value_parser=["global","project"])]
        scope: String,
    },
    Restore {
        source: PathBuf,
        target: PathBuf,
        #[arg(long)]
        fork: bool,
    },
    StoreUpgrade {
        #[arg(long,default_value="project",value_parser=["global","project"])]
        scope: String,
        #[arg(long)]
        commit: bool,
    },
    ReviseV2 {
        #[arg(long,default_value="project",value_parser=["global","project"])]
        scope: String,
    },
    WriteV2 {
        #[arg(long,default_value="project",value_parser=["global","project"])]
        scope: String,
    },
    ShowV2 {
        id: String,
        #[arg(long)]
        store_id: Option<String>,
    },
    Checkpoint {
        #[command(subcommand)]
        command: CheckpointCommand,
    },
    Config {
        #[arg(long,default_value="json",value_parser=["json"])]
        format: String,
    },
    Consolidate {
        #[arg(long,default_value="project",value_parser=["all","global","project"])]
        scope: String,
        #[arg(long, default_value_t = 0.8)]
        threshold: f64,
        #[arg(long)]
        commit: bool,
    },
    EmbedBackfill {
        #[command(flatten)]
        scope: Scope,
        #[arg(long)]
        no_archive: bool,
        #[arg(long, default_value="text", value_parser=["text","json"])]
        format: String,
    },
    Hook {
        #[arg(value_parser=["SessionStart","UserPromptSubmit","PreToolUse","Stop"])]
        event: String,
    },
    Install {
        agent: String,
        #[command(flatten)]
        options: InstallOptions,
    },
    Eval {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    Init {
        #[arg(long,default_value="generic",value_parser=["generic","codex","claude-code","grok","antigravity"])]
        agent: String,
        #[arg(long)]
        no_agent_files: bool,
    },
    Read {
        #[arg(long,default_value="project",value_parser=["all","global","project"])]
        scope: String,
    },
    Write {
        #[arg(long = "type")]
        kind: String,
        #[arg(long)]
        importance: i64,
        #[arg(long,default_value="project",value_parser=["global","project"])]
        scope: String,
        #[arg(long, default_value = "agent")]
        source: String,
        #[arg(long, default_value = "")]
        tags: String,
        #[arg(long, default_value = "")]
        title: String,
        #[arg(long, default_value = "")]
        content: String,
        #[arg(long, default_value = "")]
        expires: String,
        #[arg(long, default_value = "")]
        evidence: String,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        allow_duplicate: bool,
    },
    Search {
        query: String,
        #[arg(long)]
        as_of: Option<String>,
        #[command(flatten)]
        scope: Scope,
        #[arg(long = "type", default_value = "")]
        kind: String,
        #[arg(long, default_value_t = 5)]
        limit: usize,
        #[arg(long,default_value="text",value_parser=["text","json"])]
        format: String,
        #[arg(long)]
        archive: bool,
        #[arg(long)]
        include_superseded: bool,
    },
    History {
        id: String,
        #[arg(long)]
        store_id: Option<String>,
    },
    Show {
        id: String,
        #[arg(long)]
        revision: Option<u64>,
        #[arg(long, requires = "revision")]
        store_id: Option<String>,
    },
    Reindex {
        #[command(flatten)]
        scope: Scope,
        #[arg(long)]
        no_archive: bool,
    },
    Maintain {
        #[command(flatten)]
        scope: Scope,
        #[arg(long)]
        dry_run: bool,
    },
    Doctor {
        #[command(flatten)]
        scope: Scope,
        #[arg(long, default_value="text", value_parser=["text","json"])]
        format: String,
    },
    Link {
        id1: String,
        id2: String,
        #[arg(long, default_value = "related")]
        rel: String,
        #[arg(long)]
        allow_custom: bool,
    },
    Graph {
        id: String,
        #[arg(long, default_value_t = 1)]
        depth: usize,
        #[arg(long,default_value="mermaid",value_parser=["mermaid","ascii","json"])]
        format: String,
    },
    #[command(alias = "codex-prep")]
    Prep {
        task: String,
        #[arg(long, default_value_t = 5)]
        limit: usize,
        #[arg(long)]
        budget: Option<usize>,
        #[arg(long,default_value="text",value_parser=["text","json"])]
        format: String,
        #[arg(long)]
        task_id: Option<String>,
    },
    #[command(alias = "codex-ingest")]
    Ingest {
        #[arg(long, default_value = "codex")]
        source: String,
        #[arg(long)]
        commit: bool,
        #[arg(long,default_value="auto",value_parser=["auto","markdown","json"])]
        format: String,
    },
    Inject {
        #[arg(long,value_parser=["session_start","turn_start","file_touch","session_end"])]
        event: String,
        #[arg(long, default_value = "")]
        session: String,
        #[arg(long,default_value="cli",value_parser=["cli","mcp","none"])]
        channel: String,
        #[arg(long,default_value="text",value_parser=["text","json"])]
        format: String,
        #[arg(long)]
        fail_safe: bool,
        #[arg(long)]
        budget: Option<usize>,
        #[arg(long, default_value = "")]
        context_epoch: String,
        #[arg(long)]
        task_id: Option<String>,
    },
    Distill {
        #[arg(long, required_unless_present = "stdin", conflicts_with = "stdin")]
        transcript: Option<PathBuf>,
        #[arg(long)]
        stdin: bool,
        #[arg(long,default_value="auto",value_parser=["auto","claude-jsonl","codex-jsonl","grok-jsonl","role-jsonl","text"])]
        format: String,
        #[arg(long, default_value = "agent")]
        source: String,
        #[arg(long)]
        commit: bool,
    },
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
}
#[derive(Subcommand)]
enum CheckpointCommand {
    New,
    Load {
        id: String,
    },
    Update {
        id: String,
        #[arg(long)]
        expected_revision: u64,
    },
    Close {
        id: String,
        #[arg(long)]
        expected_revision: u64,
    },
    Observe {
        #[arg(long, value_delimiter = ',')]
        paths: Vec<String>,
    },
}
#[derive(Subcommand)]
enum McpCommand {
    Serve {
        #[arg(long)]
        sse: bool,
    },
}
fn stdin_text() -> Result<String> {
    mnemosyne::input::read(io::stdin().lock())
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli.command) {
        eprintln!("mnemosyne: {e:#}");
        std::process::exit(1);
    }
}
fn run(command: Command) -> Result<()> {
    match command {
        Command::Consolidate {
            scope,
            threshold,
            commit,
        } => println!(
            "{}",
            serde_json::to_string_pretty(&api::consolidate(
                &stores_for_scope(&scope)?,
                threshold,
                commit
            )?)?
        ),
        Command::Proposal {
            action,
            id,
            confirm_hash,
            scope,
        } => {
            let store = stores_for_scope(&scope)?.remove(0);
            let clock = mnemosyne::provenance::SystemClock;
            let result = match action.as_str() {
                "create" => mnemosyne::proposals::propose(
                    &store,
                    serde_json::from_str(&stdin_text()?)?,
                    &clock,
                )?,
                "show" => mnemosyne::proposals::show(&store, &id)?,
                _ => mnemosyne::proposals::review(&store, &id, &action, &confirm_hash, &clock)?,
            };
            println!("{}", serde_json::to_string(&result)?);
        }
        Command::Reconcile { commit, scope } => {
            let store = stores_for_scope(&scope)?.remove(0);
            let request = serde_json::from_str(&stdin_text()?)?;
            let result = if commit {
                mnemosyne::reconcile::write(&store, request, &mnemosyne::provenance::SystemClock)?
            } else {
                mnemosyne::reconcile::classify(&store, &request)?
            };
            println!("{result}");
        }
        Command::Sleep {
            action,
            cursor,
            snapshot,
            limit,
            scope,
        } => {
            let store = stores_for_scope(&scope)?.remove(0);
            let clock = mnemosyne::provenance::SystemClock;
            let result = match action.as_str() {
                "rules" => {
                    mnemosyne::sleep::rules(&store, cursor, snapshot.as_deref(), limit, &clock)?
                }
                "export" => serde_json::to_value(mnemosyne::sleep::export(
                    &store,
                    cursor,
                    snapshot.as_deref(),
                    limit,
                )?)?,
                "cursor" => mnemosyne::sleep::cursor(&store)?,
                _ => {
                    let value: Value = serde_json::from_str(&stdin_text()?)?;
                    mnemosyne::sleep::finish(
                        &store,
                        &serde_json::from_value(value["batch"].clone())?,
                        serde_json::from_value(value["proposals"].clone())?,
                        &clock,
                    )?
                }
            };
            println!("{result}");
        }
        Command::View {
            action,
            name,
            scope,
        } => {
            let store = stores_for_scope(&scope)?.remove(0);
            let result = if action == "generate" {
                mnemosyne::views::generate(
                    &store,
                    &serde_json::from_str(&stdin_text()?)?,
                    &mnemosyne::provenance::SystemClock,
                )?
            } else {
                mnemosyne::views::inspect(&store, &name)?
            };
            println!("{}", serde_json::to_string(&result)?);
        }
        Command::Snapshot { destination, scope } => println!(
            "{}",
            serde_json::to_string(&mnemosyne::snapshot::create(
                &stores_for_scope(&scope)?.remove(0),
                &destination
            )?)?
        ),
        Command::Restore {
            source,
            target,
            fork,
        } => {
            let manifest = mnemosyne::snapshot::restore(&source, &target, fork)?;
            let store = Store {
                scope: "project".into(),
                root: target,
            };
            let indexed = mnemosyne::search::reindex_store(&store, true)?;
            println!("{}", json!({"manifest":manifest,"indexed":indexed}));
        }
        Command::StoreUpgrade { scope, commit } => {
            let store = stores_for_scope(&scope)?.remove(0);
            let result = if commit {
                serde_json::to_value(mnemosyne::provenance::upgrade_store(&store)?)?
            } else {
                serde_json::to_value(mnemosyne::provenance::preview_upgrade(&store)?)?
            };
            println!("{}", result);
        }
        Command::WriteV2 { scope } => {
            let request = serde_json::from_str(&stdin_text()?)?;
            let result = mnemosyne::provenance::write_v2(
                &stores_for_scope(&scope)?.remove(0),
                &request,
                &mnemosyne::provenance::SystemClock,
            )?;
            println!("{}", serde_json::to_string(&result)?);
        }
        Command::ReviseV2 { scope } => {
            let request = serde_json::from_str(&stdin_text()?)?;
            println!(
                "{}",
                api::revise_v2(
                    &stores_for_scope(&scope)?.remove(0),
                    &request,
                    &mnemosyne::provenance::SystemClock
                )?
            );
        }
        Command::ShowV2 { id, store_id } => {
            println!(
                "{}",
                api::show_v2(&stores_for_scope("all")?, &id, store_id.as_deref())?
            );
        }
        Command::Checkpoint { command } => {
            let store = project_store();
            let clock = mnemosyne::provenance::SystemClock;
            let result = match command {
                CheckpointCommand::New => serde_json::to_value(mnemosyne::checkpoint::create(
                    &store,
                    serde_json::from_str(&stdin_text()?)?,
                    &clock,
                )?)?,
                CheckpointCommand::Load { id } => mnemosyne::checkpoint::load(&store, &id, &clock)?,
                CheckpointCommand::Update {
                    id,
                    expected_revision,
                } => serde_json::to_value(mnemosyne::checkpoint::update(
                    &store,
                    &id,
                    expected_revision,
                    serde_json::from_str(&stdin_text()?)?,
                    &clock,
                )?)?,
                CheckpointCommand::Close {
                    id,
                    expected_revision,
                } => serde_json::to_value(mnemosyne::checkpoint::close(
                    &store,
                    &id,
                    expected_revision,
                    &clock,
                )?)?,
                CheckpointCommand::Observe { paths } => {
                    serde_json::to_value(mnemosyne::checkpoint::observe(&store, &paths)?)?
                }
            };
            println!("{}", result);
        }
        Command::Config { format: _ } => {
            let c = load_config(None)?;
            let mut d = json!({});
            for key in [
                "enabled",
                "engine",
                "session_summary",
                "max_findings_per_session",
                "confidence_threshold",
            ] {
                d[key] = c["distill"][key].clone();
            }
            println!(
                "{}",
                json!({"distill":d,"memory":{"types":c["memory"]["types"]}})
            );
        }
        Command::EmbedBackfill {
            scope,
            no_archive,
            format,
        } => {
            let stores = stores_for_scope(&scope.scope)?;
            if format == "json" {
                let stats = mnemosyne::vectors::backfill_stats(&stores, !no_archive)?;
                println!("{}", serde_json::to_string(&stats)?);
                if stats.failed > 0 || stats.invalid > 0 {
                    bail!("Embedding backfill contains failed or invalid results");
                }
            } else {
                println!(
                    "Embedded {} memories",
                    mnemosyne::vectors::backfill(&stores, !no_archive)?
                );
            }
        }
        Command::Hook { event } => {
            let result = (|| -> Result<Option<Value>> {
                let text = stdin_text()?;
                let payload: Value = if text.trim().is_empty() {
                    json!({})
                } else {
                    serde_json::from_str(&text)?
                };
                mnemosyne::adapters::hook(&event, &payload)
            })();
            match result {
                Ok(Some(value)) => println!("{value}"),
                Ok(None) => {}
                Err(e) => eprintln!("mnemosyne hook: {e:#}"),
            }
        }
        Command::Install { agent, options } => install(&agent, options)?,
        Command::Eval { args } => mnemosyne::eval::run(&args)?,
        Command::Init {
            agent,
            no_agent_files,
        } => {
            let s = project_store();
            ensure_store(&s)?;
            if !no_agent_files {
                let path =
                    std::env::current_dir()?.join(if agent == "claude-code" || agent == "grok" {
                        "CLAUDE.md"
                    } else {
                        "AGENTS.md"
                    });
                if !path.exists() {
                    std::fs::write(
                        path,
                        (if agent == "claude-code" || agent == "grok" {
                            include_str!("../assets/templates/agents/claude_code/CLAUDE.md")
                        } else if agent == "codex" {
                            include_str!("../assets/templates/agents/codex/AGENTS.md")
                        } else {
                            include_str!("../assets/templates/agents/generic/AGENTS.md")
                        })
                        .replace("python3 -m mnemosyne", "mnemosyne"),
                    )?;
                }
            }
            println!("Initialized {}", s.root.display());
        }
        Command::Read { scope } => {
            for s in stores_for_scope(&scope)? {
                let core = read_core(&s)?;
                if !core.is_empty() {
                    println!("{core}");
                }
            }
        }
        Command::Write {
            kind,
            importance,
            scope,
            source,
            tags,
            title,
            content,
            expires,
            evidence,
            force: _,
            allow_duplicate,
        } => {
            let r = api::write_entry(
                &api::scope_store(&scope)?,
                &api::WriteRequest {
                    memory_type: kind,
                    importance,
                    source: source.trim().to_lowercase(),
                    tags: tags
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .collect(),
                    title,
                    content: if content.is_empty() {
                        stdin_text()?
                    } else {
                        content
                    },
                    expires,
                    evidence,
                    allow_duplicate,
                },
            )?;
            if r.status == "duplicate" {
                println!(
                    "Duplicate of {}; skipped (use --allow-duplicate to write anyway).",
                    r.id
                );
            } else {
                println!("Wrote {}", r.id);
            }
        }
        Command::Search {
            query,
            as_of,
            scope,
            kind,
            limit,
            format,
            archive,
            include_superseded,
        } => {
            if let Some(as_of) = as_of {
                let result = mnemosyne::history::search(
                    &stores_for_scope(&scope.scope)?,
                    &query,
                    &as_of,
                    limit,
                    &kind,
                    archive,
                    include_superseded,
                )?;
                // Historical output always includes coverage and date interpretation.
                println!("{}", serde_json::to_string_pretty(&result)?);
                return Ok(());
            }
            let results = api::search_entries(
                &stores_for_scope(&scope.scope)?,
                &query,
                limit,
                &kind,
                archive,
                include_superseded,
                true,
            )?;
            if format == "json" {
                println!("{}", serde_json::to_string_pretty(&results)?);
            } else if results.is_empty() {
                println!("no results");
            } else {
                for r in results {
                    println!(
                        "{} ({}/{}, {:.4}) {}",
                        r["id"].as_str().unwrap_or(""),
                        r["scope"].as_str().unwrap_or(""),
                        r["type"].as_str().unwrap_or(""),
                        r["score"].as_f64().unwrap_or(0.0),
                        r["summary"].as_str().unwrap_or("")
                    );
                }
            }
        }
        Command::History { id, store_id } => {
            println!(
                "{}",
                mnemosyne::history::list(&stores_for_scope("all")?, &id, store_id.as_deref())?
            );
        }
        Command::Show {
            id,
            revision,
            store_id,
        } => {
            if let Some(revision) = revision {
                println!(
                    "{}",
                    mnemosyne::history::show(
                        &stores_for_scope("all")?,
                        &id,
                        store_id.as_deref(),
                        revision
                    )?
                );
                return Ok(());
            }
            let (_, _, m) = find_memory(&id, &stores_for_scope("all")?, true)?
                .ok_or_else(|| anyhow::anyhow!("Memory not found: {id}"))?;
            print!("{}", serialize_memory(&m));
        }
        Command::Reindex { scope, no_archive } => {
            for s in stores_for_scope(&scope.scope)? {
                println!(
                    "{}: indexed {} memories",
                    s.scope,
                    mnemosyne::search::reindex_store(&s, !no_archive)?
                );
            }
        }
        Command::Maintain { scope, dry_run } => println!(
            "{}",
            serde_json::to_string_pretty(&api::maintain(
                &stores_for_scope(&scope.scope)?,
                dry_run
            )?)?
        ),
        Command::Doctor { scope, format } => {
            let mut failed = false;
            let mut reports = Vec::new();
            for store in stores_for_scope(&scope.scope)? {
                let canonical = load_config(Some(&store)).and_then(|_| {
                    let _guard = if store.root.exists() {
                        Some(lock_store_read_only(&store)?)
                    } else {
                        None
                    };
                    anyhow::ensure!(
                        !store.root.join(".relations-operation.json").exists(),
                        "Pending relation operation requires recovery before diagnostics"
                    );
                    load_memories_unlocked(&store, true)
                });
                let mut report =
                    json!({"scope": store.scope, "vectors": mnemosyne::vectors::diagnose(&store)});
                match canonical {
                    Ok(memories) => {
                        report["canonical"] = json!({"status":"ready","memories":memories.len()});
                        if format == "text" {
                            println!(
                                "{}: {} memories; store {}",
                                store.scope,
                                memories.len(),
                                store.root.display()
                            );
                        }
                    }
                    Err(e) => {
                        report["canonical"] = json!({"status":"error","error":e.to_string()});
                        if format == "text" {
                            eprintln!("{}: {e}", store.scope);
                        }
                        failed = true;
                    }
                }
                reports.push(report);
            }
            if format == "json" {
                println!("{}", serde_json::to_string(&reports)?);
            }
            if failed {
                bail!("Store diagnostics failed");
            }
        }
        Command::Link {
            id1,
            id2,
            rel,
            allow_custom,
        } => {
            mnemosyne::relations::link_entries(
                &stores_for_scope("all")?,
                &id1,
                &id2,
                &rel,
                allow_custom,
            )?;
            println!("Linked {id1} <-> {id2} ({rel})");
        }
        Command::Graph { id, depth, format } => println!(
            "{}",
            mnemosyne::relations::graph(&stores_for_scope("all")?, &id, depth, &format)?
        ),
        Command::Prep {
            task,
            limit,
            budget,
            format,
            task_id,
        } => {
            let stores = stores_for_scope("all")?;
            let extra = mnemosyne::checkpoint::active_context(
                &stores,
                task_id.as_deref(),
                &mnemosyne::provenance::SystemClock,
            )?;
            let bundle =
                mnemosyne::context::prep_bundle(&stores, &task, limit, "cli", budget, &extra)?;
            if format == "json" {
                println!("{}", serde_json::to_string(&bundle)?);
            } else {
                println!("{}", bundle.context);
            }
        }
        Command::Ingest {
            source,
            commit,
            format,
        } => println!(
            "{}",
            serde_json::to_string_pretty(&mnemosyne::ingest::ingest(
                &stdin_text()?,
                &source,
                commit,
                &format
            )?)?
        ),
        Command::Distill {
            transcript,
            stdin: _,
            format,
            source,
            commit,
        } => {
            let text = if let Some(path) = transcript {
                std::fs::read_to_string(path)?
            } else {
                stdin_text()?
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&mnemosyne::ingest::distill(
                    &text, &format, &source, commit
                )?)?
            );
        }
        Command::Inject {
            event,
            session,
            channel,
            format,
            fail_safe,
            budget,
            context_epoch,
            task_id,
        } => {
            let result = (|| -> Result<Value> {
                let text = stdin_text()?;
                let payload = if text.trim().is_empty() {
                    json!({})
                } else {
                    serde_json::from_str(&text)?
                };
                let extra = mnemosyne::checkpoint::active_context(
                    &stores_for_scope("all")?,
                    task_id.as_deref().or_else(|| payload["task_id"].as_str()),
                    &mnemosyne::provenance::SystemClock,
                )?;
                mnemosyne::context::inject_with_options(
                    &event,
                    &payload,
                    &session,
                    &channel,
                    budget,
                    &context_epoch,
                    &extra,
                )
            })();
            match result {
                Ok(v) => {
                    if format == "json" {
                        println!("{v}");
                    } else if let Some(s) = v["context"].as_str()
                        && !s.is_empty()
                    {
                        println!("{s}");
                    }
                }
                Err(e) if fail_safe => eprintln!("mnemosyne: {e:#}"),
                Err(e) => return Err(e),
            }
        }
        Command::Mcp {
            command: McpCommand::Serve { sse },
        } => {
            if sse {
                mnemosyne::mcp::serve_sse()?;
            } else {
                mnemosyne::mcp::serve()?;
            }
        }
    }
    Ok(())
}

fn install(agent: &str, options: InstallOptions) -> Result<()> {
    println!("{}", mnemosyne::adapters::install(agent, options.dry_run)?);
    Ok(())
}
