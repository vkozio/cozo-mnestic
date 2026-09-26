# cozo-lib-wasm web demo

Minimal Vite project that runs the CozoScript engine in the browser, installed
from the GitHub Release tarball.

```bash
npm install     # fetches cozo-lib-wasm-<version>.tar.gz from the release URL
npm run dev     # http://localhost:5173
```

The dependency is a tarball URL rather than a registry version, so the version
lives in two places that must agree — the release tag `wasm-v<VERSION>` and the
asset name `cozo-lib-wasm-<VERSION>.tar.gz`:

```json
"dependencies": {
  "cozo-lib-wasm": "https://github.com/vkozio/cozo-mnestic/releases/download/wasm-v0.18.0/cozo-lib-wasm-0.18.0.tar.gz"
}
```

`npm install` from a URL is not cached or lockfile-pinned the way a registry
version is. To pin properly, install once, then commit the `cozo-lib-wasm-<VERSION>.tar.gz`
asset into the repo and point the dependency at the relative path.

## How it is used

Three steps, all in `src/main.js`:

```js
import init, { CozoDb } from 'cozo-lib-wasm';

await init();                 // fetches + compiles the .wasm; must come first
const db = CozoDb.new();      // in-memory database
const res = JSON.parse(db.run('?[a] := a = 1', '{}', true));
```

`init()` takes no argument: the glue resolves the binary as
`new URL('cozo_lib_wasm_bg.wasm', import.meta.url)` and `fetch`es it. Under Vite
that only works because `optimizeDeps.exclude` keeps the package out of the
dependency pre-bundler, which would otherwise rewrite that module URL — see
`vite.config.js`. If the browser ever asks for a path that 404s, the fallback is
to hand the URL over explicitly:

```js
import wasmUrl from 'cozo-lib-wasm/cozo_lib_wasm_bg.wasm?url';
await init({ module_or_path: wasmUrl });
```

`run(script, params, immutable)` returns a JSON string. Success and failure share
one shape, so check `ok` before reading `rows`:

```jsonc
{ "ok": true,  "headers": ["name","age"], "rows": [["Ada", 36]] }
{ "ok": false, "code": "query::relation_not_found", "message": "..." }
```

`immutable: true` marks the script read-only and lets the engine skip the write
lock — worth using for every query that does not mutate.

## For a page without a bundler

The same package works as plain ESM — copy `cozo_lib_wasm.js` and
`cozo_lib_wasm_bg.wasm` next to each other and load the first as a module:

```html
<script type="module">
  import init, { CozoDb } from './cozo_lib_wasm.js';
  await init();
  const db = CozoDb.new();
</script>
```

It must be served over HTTP. `file://` does not work, because `init()` `fetch`es
the `.wasm` and browsers refuse `file:` URLs.

## Notes on this build

The dialect here is not upstream CozoScript: sorting is `:sort -age` rather than
`| order by age desc`, `*rel[...]` needs every column while `*rel{name: n}` binds
by name, `avg`/`sum` are missing, and `::query create` / `<~ BudgetedTraversal`
**panic** on wasm32 instead of returning an error.

All of it — with symptoms, causes and the working forms — is in
[`cozo-lib-wasm/docs/wasm-script-notes.md`](../../cozo-lib-wasm/docs/wasm-script-notes.md).
Read that before writing your own queries. The short version for this page:
`?[] | order by` does not parse, and the query box above is pre-filled with a
form that does.
