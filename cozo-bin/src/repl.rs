/*
 * Copyright 2022, The Cozo Project Authors.
 *
 * This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0.
 * If a copy of the MPL was not distributed with this file,
 * You can obtain one at https://mozilla.org/MPL/2.0/.
 */

// This file is based on code contributed by https://github.com/rhn

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::fs::File;
use std::io::{IsTerminal, Read, Write};
use std::time::Instant;

use clap::{Args, ValueEnum};
use miette::{bail, miette, IntoDiagnostic};
use rustyline::history::DefaultHistory;
use rustyline::Changeset;
use serde_json::{json, Value};

use cozo::{
    evaluate_expressions, format_error_as_json, DataValue, DbInstance, NamedRows, ScriptMutability,
    ScriptRunOptions,
};

struct Indented;

impl rustyline::hint::Hinter for Indented {
    type Hint = String;
}

impl rustyline::highlight::Highlighter for Indented {}
impl rustyline::completion::Completer for Indented {
    type Candidate = String;

    fn update(
        &self,
        _line: &mut rustyline::line_buffer::LineBuffer,
        _start: usize,
        _elected: &str,
        _cl: &mut Changeset,
    ) {
        unreachable!();
    }
}

impl rustyline::Helper for Indented {}

impl rustyline::validate::Validator for Indented {
    fn validate(
        &self,
        ctx: &mut rustyline::validate::ValidationContext<'_>,
    ) -> rustyline::Result<rustyline::validate::ValidationResult> {
        Ok(if ctx.input().starts_with(' ') {
            if ctx.input().ends_with('\n') {
                rustyline::validate::ValidationResult::Valid(None)
            } else {
                rustyline::validate::ValidationResult::Incomplete
            }
        } else {
            rustyline::validate::ValidationResult::Valid(None)
        })
    }
}

/// Shared connection flags (engine/path/config), flattened into `repl` and
/// `exec` so both subcommands accept identical `-e/-p/-c` flags.
#[derive(Args, Debug, Clone)]
pub(crate) struct DbConnectArgs {
    /// Database engine, can be `mem`, `sqlite`, `redb`, `rocksdb` and others.
    #[clap(short, long, default_value_t = String::from("mem"))]
    pub(crate) engine: String,

    /// Path to the database file. Ignored for the `mem` engine.
    #[clap(short, long, default_value_t = String::from("cozo.db"))]
    pub(crate) path: String,

    /// Extra config in JSON format
    #[clap(short, long, default_value_t = String::from("{}"))]
    pub(crate) config: String,
}

/// Output format for query results.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputFormat {
    /// Human-readable tables (the historical REPL default).
    Table,
    /// Machine-readable JSON on stdout; diagnostics on stderr.
    Json,
}

/// Interactive Cozo REPL (human-readable tables by default).
#[derive(Args, Debug)]
pub(crate) struct ReplArgs {
    #[clap(flatten)]
    pub(crate) db: DbConnectArgs,

    /// Output format for results (`table` keeps the historical interactive
    /// behaviour). With `json`, result payloads go to stdout as JSON and all
    /// diagnostics go to stderr.
    #[clap(long, value_enum)]
    pub(crate) format: Option<OutputFormat>,

    /// Suppress banners; informational messages go to stderr instead of stdout.
    #[clap(long)]
    pub(crate) quiet: bool,

    /// Path to the REPL history file. By default `<db-path>.history` next to
    /// the database file (never the CWD); no history file is used for `mem`.
    #[clap(long)]
    pub(crate) history_file: Option<String>,

    /// Disable the REPL history file entirely.
    #[clap(long, conflicts_with = "history_file")]
    pub(crate) no_history: bool,
}

/// One-shot CozoScript execution for agents (JSON output, exit code reflects success).
#[derive(Args, Debug)]
pub(crate) struct ExecArgs {
    #[clap(flatten)]
    pub(crate) db: DbConnectArgs,

    /// CozoScript to run, given inline. Exactly one of `--cmd` / `--file` is
    /// required. A call runs a single script with a single stored-relation op;
    /// multi-step flows need several invocations (the database is persistent).
    #[clap(long)]
    pub(crate) cmd: Option<String>,

    /// Run the whole file as a single CozoScript (same single-stored-op rule
    /// as `--cmd`: keep schema and `:put` in separate files/calls).
    #[clap(long, conflicts_with = "cmd")]
    pub(crate) file: Option<String>,

    /// Output format for the result.
    #[clap(long, value_enum, default_value_t = OutputFormat::Json)]
    pub(crate) format: OutputFormat,

    /// Per-call query timeout in seconds (forwarded to `ScriptRunOptions::timeout`).
    #[clap(long)]
    pub(crate) timeout: Option<f64>,
}

fn open_db(db: &DbConnectArgs) -> miette::Result<DbInstance> {
    DbInstance::new(&db.engine, &db.path, &db.config)
}

/// Default history location: next to the database file, never the CWD.
/// The `mem` engine (and empty/`:memory:` paths) gets no history file.
fn default_history_file(db: &DbConnectArgs) -> Option<String> {
    if db.engine == "mem" {
        return None;
    }
    let p = db.path.trim();
    if p.is_empty() || p == ":memory:" {
        return None;
    }
    Some(format!("{p}.history"))
}

/// Informational messages: stdout for the classic interactive table REPL,
/// stderr whenever output must stay machine-readable (`json`) or quiet.
fn info_print(format: OutputFormat, quiet: bool, msg: String) {
    if format == OutputFormat::Json || quiet {
        eprintln!("{msg}");
    } else {
        println!("{msg}");
    }
}

fn print_table(out: &NamedRows) -> miette::Result<()> {
    use prettytable::format;
    let mut table = prettytable::Table::new();
    let headers = out
        .headers
        .iter()
        .map(prettytable::Cell::from)
        .collect::<Vec<_>>();
    table.set_titles(prettytable::Row::new(headers));
    let rows = out
        .rows
        .iter()
        .map(|r| r.iter().map(|c| format!("{c}")).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let rows = rows
        .iter()
        .map(|r| r.iter().map(prettytable::Cell::from).collect::<Vec<_>>());
    for row in rows {
        table.add_row(prettytable::Row::new(row));
    }
    table.set_format(*format::consts::FORMAT_NO_BORDER_LINE_SEPARATOR);
    table.printstd();
    Ok(())
}

fn row_of(vals: &[String]) -> prettytable::Row {
    prettytable::Row::new(vals.iter().map(prettytable::Cell::from).collect())
}

fn emit_rows(
    out: NamedRows,
    save_next: &mut Option<String>,
    format: OutputFormat,
    quiet: bool,
    took_secs: f64,
) -> miette::Result<()> {
    if let Some(path) = save_next.as_ref() {
        info_print(
            format,
            quiet,
            format!(
                "Query has returned {} rows, saving to file {}",
                out.rows.len(),
                path
            ),
        );

        let to_save = out
            .rows
            .iter()
            .map(|row| -> Value {
                row.iter()
                    .zip(out.headers.iter())
                    .map(|(v, k)| (k.to_string(), v.clone()))
                    .collect()
            })
            .collect();

        let j_payload = Value::Array(to_save);

        let mut file = File::create(path).into_diagnostic()?;
        file.write_all(j_payload.to_string().as_bytes())
            .into_diagnostic()?;
        *save_next = None;
    } else {
        match format {
            OutputFormat::Table => print_table(&out)?,
            OutputFormat::Json => {
                let mut j = out.into_json();
                let map = j.as_object_mut().expect("NamedRows JSON is an object");
                map.insert("ok".to_string(), json!(true));
                map.insert("took".to_string(), json!(took_secs));
                println!("{j}");
            }
        }
    }
    Ok(())
}

pub(crate) fn repl_main(args: ReplArgs) -> Result<(), Box<dyn Error>> {
    let db = open_db(&args.db)?;

    let db_copy = db.clone();
    ctrlc::set_handler(move || {
        let running = db_copy
            .run_default("::running")
            .expect("Cannot determine running queries");
        for row in running.rows {
            let id = row.into_iter().next().unwrap();
            eprintln!("Killing running query {id}");
            db_copy
                .run_script(
                    "::kill $id",
                    BTreeMap::from([("id".to_string(), id)]),
                    ScriptMutability::Mutable,
                )
                .expect("Cannot kill process");
        }
    })
    .expect("Error setting Ctrl-C handler");

    let stdin_tty = std::io::stdin().is_terminal();
    let stdout_tty = std::io::stdout().is_terminal();
    let format = args.format.unwrap_or(OutputFormat::Table);
    if args.format.is_none() && !stdin_tty {
        eprintln!(
            "warning: stdin is not a TTY; output stays as `table` (interactive default unchanged). \
             Use `--format json` for machine-readable output."
        );
    }

    if !args.quiet && stdin_tty && stdout_tty {
        println!("Welcome to the Cozo REPL.");
        println!("Type a space followed by newline to enter multiline mode.");
    }

    let history_path: Option<String> = if args.no_history {
        None
    } else {
        args.history_file
            .clone()
            .or_else(|| default_history_file(&args.db))
    };

    let mut exit = false;
    let mut rl = rustyline::Editor::<Indented, DefaultHistory>::new()?;
    let mut params = BTreeMap::new();
    let mut save_next: Option<String> = None;
    rl.set_helper(Some(Indented));

    if let Some(hf) = history_path.as_deref() {
        if rl.load_history(hf).is_ok() {
            eprintln!("Loaded history from {hf}");
        }
    }

    loop {
        let readline = rl.readline("=> ");
        match readline {
            Ok(line) => {
                if let Err(err) = process_line(
                    &line,
                    &db,
                    &mut params,
                    &mut save_next,
                    format,
                    args.quiet,
                    &args.db.path,
                ) {
                    eprintln!("{err:?}");
                }
                if let Err(err) = rl.add_history_entry(line) {
                    eprintln!("{err:?}");
                }
                exit = false;
            }
            Err(rustyline::error::ReadlineError::Interrupted) => {
                if exit {
                    break;
                } else {
                    println!("Again to exit");
                    exit = true;
                }
            }
            Err(rustyline::error::ReadlineError::Eof) => break,
            Err(e) => eprintln!("{e:?}"),
        }
    }

    if let Some(hf) = history_path.as_deref() {
        if rl.save_history(hf).is_ok() {
            eprintln!("Query history saved in {hf}");
        }
    }
    Ok(())
}

/// One-shot script execution for agents: print the result, exit non-zero on error.
/// Stdin is deliberately ignored (piped input is never read).
pub(crate) fn exec_main(args: ExecArgs) -> miette::Result<()> {
    let script = match (&args.cmd, &args.file) {
        (Some(cmd), None) => cmd.clone(),
        (None, Some(path)) => fs::read_to_string(path).into_diagnostic()?,
        (Some(_), Some(_)) => bail!("--cmd and --file are mutually exclusive"),
        (None, None) => bail!("one of --cmd or --file is required"),
    };
    if let Some(t) = args.timeout {
        if !(t > 0.0) {
            bail!("--timeout must be a positive number of seconds");
        }
    }
    let db = open_db(&args.db)?;
    let options = ScriptRunOptions {
        timeout: args.timeout,
        ..Default::default()
    };
    let start = Instant::now();
    match db.run_script_with_options(&script, BTreeMap::new(), ScriptMutability::Mutable, options)
    {
        Ok(out) => {
            match args.format {
                OutputFormat::Table => print_table(&out)?,
                OutputFormat::Json => {
                    let mut j = out.into_json();
                    let map = j.as_object_mut().expect("NamedRows JSON is an object");
                    map.insert("ok".to_string(), json!(true));
                    map.insert("took".to_string(), json!(start.elapsed().as_secs_f64()));
                    println!("{j}");
                }
            }
            Ok(())
        }
        Err(err) => {
            match args.format {
                OutputFormat::Json => {
                    let j = format_error_as_json(err, Some(script.as_str()));
                    eprintln!("{j}");
                }
                OutputFormat::Table => {
                    eprintln!("{err:?}");
                }
            }
            // Exit here so the error JSON on stderr stays the only output:
            // returning Err would make main() print a second line.
            std::process::exit(1);
        }
    }
}

struct RelStat {
    name: String,
    columns: usize,
    indices: usize,
    rows: i64,
}

/// `%stats`: one row per user relation (index relations `a:b` are skipped)
/// with column/index counts and a server-side row count. The count query is
/// assembled from the relation arity reported by `::relations`, so no new
/// sysop is needed. Also reports the on-disk DB file size and a TOTAL row.
fn emit_stats(
    db: &DbInstance,
    params: BTreeMap<String, DataValue>,
    db_path: &str,
    save_next: &mut Option<String>,
    format: OutputFormat,
    quiet: bool,
) -> miette::Result<()> {
    let rels = db.run_script("::relations", params.clone(), ScriptMutability::Mutable)?;
    let name_pos = rels.headers.iter().position(|h| h == "name").unwrap_or(0);
    let arity_pos = rels.headers.iter().position(|h| h == "arity");
    let mut stats = Vec::new();
    for row in &rels.rows {
        let name = row
            .get(name_pos)
            .and_then(|v| v.get_str())
            .unwrap_or("")
            .to_string();
        if name.is_empty() || name.contains(':') {
            continue;
        }
        let arity = arity_pos
            .and_then(|p| row.get(p))
            .and_then(|v| v.get_int())
            .unwrap_or(0)
            .max(0) as usize;
        let cols =
            db.run_script(&format!("::columns {name}"), params.clone(), ScriptMutability::Mutable)?;
        let idxs =
            db.run_script(&format!("::indices {name}"), params.clone(), ScriptMutability::Mutable)?;
        let n_rows = if arity == 0 {
            0
        } else {
            let vars: Vec<String> = (0..arity).map(|i| format!("v{i}")).collect();
            let q = format!("?[count(v0)] := *{name}[{}]", vars.join(", "));
            let out = db.run_script(&q, params.clone(), ScriptMutability::Mutable)?;
            out.rows
                .first()
                .and_then(|r| r.first())
                .and_then(|v| v.get_int())
                .unwrap_or(0)
        };
        stats.push(RelStat {
            name,
            columns: cols.rows.len(),
            indices: idxs.rows.len(),
            rows: n_rows,
        });
    }
    stats.sort_by(|a, b| a.name.cmp(&b.name));
    let total: i64 = stats.iter().map(|s| s.rows).sum();
    let db_bytes: Option<u64> = fs::metadata(db_path)
        .ok()
        .filter(|m| m.is_file())
        .map(|m| m.len());
    let payload = json!({
        "tables": stats
            .iter()
            .map(|s| json!({"relation": s.name, "columns": s.columns, "indices": s.indices, "rows": s.rows}))
            .collect::<Vec<_>>(),
        "total_rows": total,
        "db_path": db_path,
        "db_file_bytes": db_bytes,
    });
    if let Some(path) = save_next.as_ref() {
        info_print(
            format,
            quiet,
            format!("Stats for {} relations, saving to file {path}", stats.len()),
        );
        let mut file = File::create(path).into_diagnostic()?;
        file.write_all(payload.to_string().as_bytes())
            .into_diagnostic()?;
        *save_next = None;
        return Ok(());
    }
    match format {
        OutputFormat::Json => {
            let mut j = payload;
            let map = j.as_object_mut().expect("stats JSON is an object");
            map.insert("ok".to_string(), json!(true));
            println!("{j}");
        }
        OutputFormat::Table => {
            use prettytable::format as tbl_format;
            let mut table = prettytable::Table::new();
            table.set_titles(row_of(&[
                "relation".to_string(),
                "columns".to_string(),
                "indices".to_string(),
                "rows".to_string(),
            ]));
            for s in &stats {
                table.add_row(row_of(&[
                    s.name.clone(),
                    s.columns.to_string(),
                    s.indices.to_string(),
                    s.rows.to_string(),
                ]));
            }
            table.add_row(row_of(&[
                "TOTAL".to_string(),
                String::new(),
                String::new(),
                total.to_string(),
            ]));
            table.set_format(*tbl_format::consts::FORMAT_NO_BORDER_LINE_SEPARATOR);
            table.printstd();
            match db_bytes {
                Some(b) => info_print(
                    format,
                    quiet,
                    format!("Database file {db_path}: {b} bytes"),
                ),
                None => info_print(
                    format,
                    quiet,
                    format!(
                        "Database path {db_path}: size unavailable (in-memory or not a file)"
                    ),
                ),
            }
        }
    }
    Ok(())
}

fn process_line(
    line: &str,
    db: &DbInstance,
    params: &mut BTreeMap<String, DataValue>,
    save_next: &mut Option<String>,
    format: OutputFormat,
    quiet: bool,
    db_path: &str,
) -> miette::Result<()> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(());
    }
    let start = Instant::now();

    if let Some(remaining) = line.strip_prefix('%') {
        let remaining = remaining.trim();
        let (op, payload) = remaining
            .split_once(|c: char| c.is_whitespace())
            .unwrap_or((remaining, ""));
        match op {
            "eval" => {
                let out = evaluate_expressions(payload, params, params)?;
                println!("{out}");
            }
            "set" => {
                let (key, v_str) = payload
                    .trim()
                    .split_once(|c: char| c.is_whitespace())
                    .ok_or_else(|| miette!("Bad set syntax. Should be '%set <KEY> <VALUE>'."))?;
                let val: Value = serde_json::from_str(v_str).into_diagnostic()?;
                let val = DataValue::from(val);
                params.insert(key.to_string(), val);
            }
            "unset" => {
                let key = payload.trim();
                if params.remove(key).is_none() {
                    bail!("Key not found: '{}'", key)
                }
            }
            "clear" => {
                params.clear();
            }
            "params" => {
                let display = serde_json::to_string_pretty(&json!(&params)).into_diagnostic()?;
                println!("{display}");
            }
            "backup" => {
                let path = payload.trim();
                if path.is_empty() {
                    bail!("Backup requires a path");
                };
                db.backup_db(path)?;
                info_print(
                    format,
                    quiet,
                    format!("Backup written successfully to {path}"),
                )
            }
            "run" => {
                let path = payload.trim();
                if path.is_empty() {
                    bail!("Run requires path to a script");
                }
                let content = fs::read_to_string(path).into_diagnostic()?;
                let out = db.run_script(&content, params.clone(), ScriptMutability::Mutable)?;
                emit_rows(out, save_next, format, quiet, start.elapsed().as_secs_f64())?;
            }
            "restore" => {
                let path = payload.trim();
                if path.is_empty() {
                    bail!("Restore requires a path");
                };
                db.restore_backup(path)?;
                info_print(
                    format,
                    quiet,
                    format!("Backup successfully loaded from {path}"),
                )
            }
            "save" => {
                let next_path = payload.trim();
                if next_path.is_empty() {
                    info_print(format, quiet, "Next result will NOT be saved to file".to_string());
                } else {
                    info_print(
                        format,
                        quiet,
                        format!("Next result will be saved to file: {next_path}"),
                    );
                    *save_next = Some(next_path.to_string())
                }
            }
            "import" => {
                let url = payload.trim();
                if url.starts_with("http://") || url.starts_with("https://") {
                    let data = minreq::get(url).send().into_diagnostic()?;
                    let data = data.as_str().into_diagnostic()?;
                    db.import_relations_str_with_err(data)?;
                    info_print(format, quiet, format!("Imported data from {url}"))
                } else {
                    let file_path = url.strip_prefix("file://").unwrap_or(url);
                    let mut file = File::open(file_path).into_diagnostic()?;
                    let mut content = String::new();
                    file.read_to_string(&mut content).into_diagnostic()?;
                    db.import_relations_str_with_err(&content)?;
                    info_print(format, quiet, format!("Imported data from {url}"));
                }
            }
            "tables" => {
                if !payload.trim().is_empty() {
                    bail!("`%tables` takes no arguments (it runs `::relations`)");
                }
                let out = db.run_script("::relations", params.clone(), ScriptMutability::Mutable)?;
                emit_rows(out, save_next, format, quiet, start.elapsed().as_secs_f64())?;
            }
            "schema" => {
                let rel = payload.trim();
                if rel.is_empty() {
                    bail!("`%schema` requires a relation name, e.g. `%schema twiggy`");
                }
                let cols = db.run_script(
                    &format!("::columns {rel}"),
                    params.clone(),
                    ScriptMutability::Mutable,
                )?;
                let idxs = db.run_script(
                    &format!("::indices {rel}"),
                    params.clone(),
                    ScriptMutability::Mutable,
                )?;
                if save_next.is_some() {
                    let path = save_next.as_ref().expect("checked above").clone();
                    info_print(
                        format,
                        quiet,
                        format!("Schema for relation {rel}, saving to file {path}"),
                    );
                    let j = json!({
                        "relation": rel,
                        "columns": cols.into_json(),
                        "indices": idxs.into_json(),
                    });
                    let mut file = File::create(&path).into_diagnostic()?;
                    file.write_all(j.to_string().as_bytes())
                        .into_diagnostic()?;
                    *save_next = None;
                } else if format == OutputFormat::Json {
                    let j = json!({
                        "ok": true,
                        "relation": rel,
                        "columns": cols.into_json(),
                        "indices": idxs.into_json(),
                        "took": start.elapsed().as_secs_f64(),
                    });
                    println!("{j}");
                } else {
                    emit_rows(cols, save_next, format, quiet, start.elapsed().as_secs_f64())?;
                    emit_rows(idxs, save_next, format, quiet, start.elapsed().as_secs_f64())?;
                }
            }
            "stats" => {
                if !payload.trim().is_empty() {
                    bail!("`%stats` takes no arguments");
                }
                emit_stats(db, params.clone(), db_path, save_next, format, quiet)?;
            }
            "graphs" => {
                if !payload.trim().is_empty() {
                    bail!("`%graphs` takes no arguments (it runs `::graph list`)");
                }
                let out =
                    db.run_script("::graph list", params.clone(), ScriptMutability::Mutable)?;
                emit_rows(out, save_next, format, quiet, start.elapsed().as_secs_f64())?;
            }
            "queries" => {
                if !payload.trim().is_empty() {
                    bail!("`%queries` takes no arguments (it runs `::query list`)");
                }
                let out =
                    db.run_script("::query list", params.clone(), ScriptMutability::Mutable)?;
                emit_rows(out, save_next, format, quiet, start.elapsed().as_secs_f64())?;
            }
            _ => {
                let out = db.run_script(line, params.clone(), ScriptMutability::Mutable)?;
                emit_rows(out, save_next, format, quiet, start.elapsed().as_secs_f64())?;
            }
        }
    } else {
        let out = db.run_script(line, params.clone(), ScriptMutability::Mutable)?;
        emit_rows(out, save_next, format, quiet, start.elapsed().as_secs_f64())?;
    }
    Ok(())
}
