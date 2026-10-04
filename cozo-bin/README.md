# mnestic standalone server / REPL (cozo-bin)

> Part of **mnestic**, an independently maintained fork of CozoDB; **not** official CozoDB. See [`../FORK.md`](../FORK.md). Original design © Ziyang Hu and the Cozo Project Authors.

This document describes how to set up cozo (standalone executable).
To learn how to use CozoDB (CozoScript), read the [docs](https://docs.cozodb.org/en/latest/index.html).

## Download

mnestic does not currently publish prebuilt binaries. Build from source as described in the Building section below (or run with `cargo run -p cozo-bin`).

## Starting the server

Run the cozo command in a terminal:

```bash
./cozo server
```

This starts an in-memory, non-persistent database.
For more options such as how to run a persistent database with other storage engines,
see `./cozo server -h`

To stop Cozo, press `CTRL-C`, or send `SIGTERM` to the process with e.g. `kill`.

## The REPL

Run `./cozo repl` to enter a terminal-based REPL. The engine options can be used when
invoking the executable to choose the backend.

You can use the following meta ops in the REPL:

* `%set <KEY> <VALUE>`: set a parameter that can be used in queries.
* `%unset <KEY>`: unset a parameter.
* `%clear`: unset all parameters.
* `%params`: print all set parameters.
* `%run <FILE>`: run the script contained in `<FILE>`.
* `%import <FILE OR URL>`: import data in JSON format from the file or URL.
* `%save <FILE>`: the result of the next successful query will be saved in JSON format in a file instead of printed on
  screen. If `<FILE>` is omitted, then the effect of any previous `%save` command is nullified.
* `%backup <FILE>`: the current database will be backed up into the file.
* `%restore <FILE>`: restore the data in the backup to the current database. The current database must be empty.

## Non-interactive scripts (`exec`)

`repl` is built for a human at a terminal: prompt, history file, banner. For scripts, agents
and CI use `./cozo exec`, which runs one CozoScript, prints the result and exits with a status
code.

| mode   | use when                                            | output default |
| ------ | --------------------------------------------------- | -------------- |
| `exec` | one-shot script, machine-readable result, exit code | `json`         |
| `repl` | interactive exploration, `%`-meta commands, pipes   | `table`        |

`exec` and `repl` take the same connection flags:

```
-e, --engine <ENGINE>   mem | sqlite | redb | rocksdb | ...
-p, --path <PATH>       DB file. Ignored for mem. Default: cozo.db
-c, --config <CONFIG>   extra config as JSON. Default: {}
```

`exec` flags:

```
--cmd <CMD>         CozoScript inline. Exactly one of --cmd / --file is required.
--file <FILE>       Run the whole file as a single CozoScript
                    (same single-stored-op rule as --cmd).
--format <FORMAT>   table | json  [default: json]
--timeout <TIMEOUT> per-call query timeout in seconds, must be > 0
```

`repl` flags:

```
--format <FORMAT>               table | json (no default = table interactively)
--quiet                         suppress banners; infos go to stderr
--history-file <HISTORY_FILE>   explicit history path
--no-history                    disable history (conflicts with --history-file)
```

A database file persists across calls, so multi-step flows are several `exec` invocations —
see the one-op rule below.

```bash
./cozo exec -e redb -p data.redb --cmd ':create item {id: Int, name: String}'
./cozo exec -e redb -p data.redb --cmd ':put item <- [{id: 1, name: "bolt"}, {id: 2, name: "nut"}]'
./cozo exec -e redb -p data.redb --cmd '?[count(id)] := *item[id, _]'
```

```json
{"headers":["count(id)"],"rows":[[2]],"next":null,"ok":true,"took":0.048}
```

The same query for a human (`--format table`):

```
 count(id)
 -----------
  2
```

System catalogs are plain scripts, so they work in `exec` (`%`-commands do not):

```bash
./cozo exec -e redb -p data.redb --cmd '::columns item'
```

```json
{
  "headers": ["column", "is_key", "index", "type", "has_default", "default_expr"],
  "next": null,
  "ok": true,
  "rows": [
    ["id", true, 0, "Int", false, null],
    ["name", false, 1, "String", false, null]
  ],
  "took": 0.017
}
```

`--file` runs a whole file as one script:

```bash
./cozo exec -e redb -p data.redb --file q.cozo
```

To drive the `%`-meta commands from a script, pipe lines into `repl`; one line is one command
and `--format json` prints one JSON object per line. Always pass `--no-history` and `--quiet`
in automation.

```bash
echo '%tables' | ./cozo repl -e redb -p data.redb --format json --no-history --quiet
echo '%stats'  | ./cozo repl -e redb -p data.redb --format table --no-history --quiet
```

`%tables`, `%schema <rel>`, `%stats`, `%eval`, `%run`, `%graphs`, `%queries` and the
parameter commands (`%set`, `%unset`, `%clear`, `%params`) are REPL-only; an unknown `%foo`
falls through to `run_script`.

### Output contract

On success the payload goes to stdout and the exit code is 0:

```json
{"headers": ["<col>", "..."], "rows": [[...]], "next": null, "ok": true, "took": 0.1}
```

`took` is seconds (float) and varies per run; do not assert on it. `%schema` and `%stats` have
their own shapes (`{"ok":true,"relation":"...","columns":{...},"indices":{...}}` and
`{"ok":true,"tables":[...],"total_rows":N,"db_path":"...","db_file_bytes":N}`).

On failure stdout stays empty, one JSON object (or miette text with `--format table`) goes to
stderr:

```json
{
  "causes": [],
  "code": "query::relation_not_found",
  "display": "...",
  "filename": "",
  "labels": [],
  "message": "Cannot find requested stored relation 'nosuch'",
  "ok": false,
  "related": [],
  "severity": "error"
}
```

Error keys: `ok:false`, `code` (`parser::pest`, `query::relation_not_found`,
`eval::no_implementation`, ...), `message`, `display` (ANSI-colored), `labels`, `causes`,
`related`, `severity`, `filename`, and `help` for parser errors.

### Exit codes

| code    | meaning                                                                                |
| ------- | -------------------------------------------------------------------------------------- |
| 0       | success                                                                                |
| 1       | script ran, query failed                                                                |
| 2       | CLI usage error from clap                                                              |
| -1 (255 on Unix shells) | infrastructure error: no `--cmd`/`--file`, `--timeout 0`, unreadable `--file`, unknown engine |

### Limits and gotchas

1. One stored op per call. Schema and data never share one `--cmd`/`--file`/`%run`: a script
   with both `:create` and `:put` fails with `parser::pest`. Use one call per op.
2. Aggregations only in head position. `?[sum(x)] := *rel[_, x, _]` works; `?[x] := x = sum(1)`
   fails with `eval::no_implementation`.
3. `count()` with no argument is a parser error, not an empty count. Use `count(id)` with a
   bound variable.
4. `exec` ignores stdin entirely; only `--cmd` / `--file` runs. `repl` reads stdin line by line.
5. REPL history defaults to `<db-path>.history` next to the database file, never the working
   directory; `mem` and `:memory:` get no history.
6. Without a TTY, `repl` keeps the `table` default and warns on stderr. Pass `--format json`
   explicitly in automation.
7. `--timeout` is seconds (float) and must be `> 0`.
8. `sum()` over ints returns a float in JSON (`2.0`) but prints as an int in tables (`2`).
   Compare numerically, not textually.

## The query API

Queries are run by sending HTTP POST requests to the server.
By default, the API endpoint is `http://127.0.0.1:9070/text-query`.
A JSON body of the following form is expected:

```json
{
  "script": "<COZOSCRIPT QUERY STRING>",
  "params": {}
}
```

params should be an object of named parameters. For example, if params is `{"num": 1}`,
then `$num` can be used anywhere in your query string where an expression is expected.
Always use params instead of concatenating strings when you need parametrized queries.

The HTTP API always responds in JSON. If a request is successful, then its `"ok"` field will be `true`,
and the `"rows"` field will contain the data for the resulting relation, and `"headers"` will contain
the headers. If an error occurs, then `"ok"` will contain `false`, the error message will be in `"message"`
and a nicely-formatted diagnostic will be in `"display"` if available.

> Cozo is designed to run in a trusted environment and be used by trusted clients.
> It does not come with elaborate authentication and security features.
> If you must access Cozo remotely, you are responsible for setting up firewalls, encryptions and proxies yourself.
>
> As a guard against users accidentally exposing sensitive data,
> Cozo generates a token string and requires all queries
> to provide the token string in the HTTP header field `x-cozo-auth`.
> The startup log tells you where to find the token string.
> Authentication may be disabled explicitly with `--insecure-no-auth`,
> but only when binding to a loopback address.
> This “security measure” is not considered sufficient for any purpose
> and is only intended as a last defence against carelessness.
>
> In some environments, setting the header may be difficult or impossible
> for some of the APIs. In this case you can pass the token in the query parameter `auth`.

## API

* `POST /text-query`, described above.
* `GET /export/{relations: String}`, where `relations` is a comma-separated list of relations to export.
* `PUT /import`, import data into the database. Data should be in `application/json` MIME type in the body,
  in the same format as returned in the `data` field in the `/export` API.
* `POST /backup`, backup database, should supply a JSON body of the form `{"path": <PATH>}`.
  The path is relative to the server's `--backup-dir` (default: `backups`) and cannot escape it.
* `POST /import-from-backup`, import data into the database from a backup. Should supply a JSON body
  of the form `{"path": <PATH>, "relations": <ARRAY OF RELATION NAMES>}`. The same
  `--backup-dir` restriction applies.
* `GET /`, if you open this in your browser and open your developer tools, you will be able to use
  a very simple client to query this database.

> For `import` and `import-from-backup`, triggers are _not_ run for the relations, if any exists.
> If you need to activate triggers, use queries with parameters.

The following are experimental:

* `GET(SSE) /changes/{relation: String}` get changes when mutations are made against a relation, relies
  on [SSE](https://developer.mozilla.org/en-US/docs/Web/API/Server-sent_events/Using_server-sent_events).

## Building

Building `cozo` requires a [Rust toolchain](https://rustup.rs). Run

```bash
cargo build --release -p cozo-bin -F compact -F storage-rocksdb
```

`cozo-bin` builds the engine with `default-features = false`, so every engine feature it wants
has to be named. The `max` preset is a shortcut for the widest build — redb, all readers, the
Cypher surface and the full FTS stack:

```bash
cargo build --release -p cozo-bin -F max
```
