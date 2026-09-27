// Temporary trap probe: one case per child process, fresh CozoDb per process
// (docs/wasm-script-notes.md, "Verification"). Usage:
//   node probe.mjs <pkg-dir> <case>
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { join } from 'node:path';

const pkg = process.argv[2];
const caseName = process.argv[3];

const HNSW = '::hnsw create pts:idx {dim: 2, m: 16, dtype: F32, fields: [emb], distance: L2, ef_construction: 50}';

const CASES = {
  basic: ['?[a] := a = 1'],
  query_create: ['::query create adults { ?[a] := a = 1 }'],
  hnsw_empty: [':create pts {id: Int => emb: <F32; 2>}', HNSW],
  hnsw_rows: [
    ':create pts {id: Int => emb: <F32; 2>}',
    '?[id, emb] <- [[1, [1.0, 0.0]], [2, [2.0, 0.0]], [3, [3.0, 0.0]]] :put pts {id => emb}',
    HNSW,
  ],
  hnsw_query: [
    ':create pts {id: Int => emb: <F32; 2>}',
    '?[id, emb] <- [[1, [1.0, 0.0]], [2, [2.0, 0.0]]] :put pts {id => emb}',
    '?[id, dist] := ~pts:idx{id | query: vec([1.0, 0.0]), k: 2, bind_distance: dist}',
  ],
  lsh_rows: [
    ':create doc {id: Int => body: String}',
    "?[id, body] <- [[1, 'the quick brown fox jumps over the lazy dog'], [2, 'a completely different text about databases']] :put doc {id => body}",
    '::lsh create doc:idx {extractor: body, tokenizer: Simple}',
    '?[id] := ~doc:idx{id | query: "the quick brown fox jumps over the lazy dog", k: 5}',
  ],
  degree_centrality: [
    'e[a, b] <- [[1, 2], [2, 3]]\n?[node, d, i, o] <~ DegreeCentrality(e[a, b])',
  ],
  dfs: [
    'e[a, b] <- [[1, 2], [2, 3]]\ns[x] <- [[1], [2]]\n?[] <~ DFS(e[a, b], s[x], condition: (x > 0))',
  ],
  random_walk: [
    'e[a, b] <- [[1, 2], [2, 3]]\ns[x] <- [[1], [2], [3]]\nt[y] <- [[1], [2], [3]]\n?[] <~ RandomWalk(e[a, b], s[x], t[y], steps: 3)',
  ],
  astar: [
    'e[a, b] <- [[1, 2], [2, 3]]\nn[z] <- [[1], [2], [3]]\ns[x] <- [[1]]\ng[y] <- [[3]]\n?[] <~ ShortestPathAStar(e[a, b], n[z], s[x], g[y], heuristic: 1)',
  ],
  hnsw_incremental: [
    ':create pts {id: Int => emb: <F32; 2>}',
    HNSW,
    '?[id, emb] <- [[1, [1.0, 0.0]], [2, [2.0, 0.0]], [3, [3.0, 0.0]]] :put pts {id => emb}',
    '?[id, dist] := ~pts:idx{id | query: vec([1.0, 0.0]), k: 2, ef: 80, bind_distance: dist} :order dist',
  ],
  fts_rows: [
    ':create doc {id: Int => body: String}',
    "?[id, body] <- [[1, 'the quick brown fox']] :put doc {id => body}",
    '::fts create doc:fts {extractor: body, tokenizer: Simple}',
    '?[id, score] := ~doc:fts{id | query: "quick", k: 5, bind_score: score}',
  ],
  pagerank: ['e[a, b] <- [[1, 2], [2, 3], [3, 1]]\n?[node, rank] <~ PageRank(e[a, b])'],
  top_sort: ['e[a, b] <- [[1, 2], [2, 3]]\n?[a, b] <~ TopSort(e[a, b])'],
  bfs: [
    'e[a, b] <- [[1, 2], [2, 3]]\ns[x] <- [[1], [2]]\n?[a, b, c] <~ BFS(e[a, b], s[x], condition: (x > 0))',
  ],
  spbfs: [
    'e[a, b] <- [[1, 2], [2, 3]] s[n] <- [[1]] g[m] <- [[3]]\n?[a, b, c] <~ ShortestPathBFS(e[x, y], s[n], g[m])',
  ],
  spbfs_timeout: [
    'e[a, b] <- [[1, 2], [2, 3]] s[n] <- [[1]] g[m] <- [[3]]\n?[a, b, c] <~ ShortestPathBFS(e[x, y], s[n], g[m]) :timeout 0.000001',
  ],
  triggers: [
    ':create src {k: Int => v: Int}',
    "::set_triggers src on put { ?[k, v] := _new[k, v] :put audit {k => v} }",
    '::show_triggers src',
  ],
  graph: [':create e {a: Int, b: Int}', '::graph create g1 {edges: e}', '::graph list'],
  graph_drop: [':create e {a: Int, b: Int}', '::graph create g1 {edges: e}', '::graph drop g1'],
  graph_query: [
    ':create e {a: Int, b: Int}',
    '?[a, b] <- [[1, 2], [2, 3]] :put e {a, b}',
    '::graph create g1 {edges: e}',
    "?[node, rank] <~ PageRank(graph: 'g1')",
  ],
};

const steps = CASES[caseName];
if (!steps) {
  console.error('unknown case:', caseName, 'known:', Object.keys(CASES).join(','));
  process.exit(2);
}

const require = createRequire(import.meta.url);
const mod = require(join(pkg, 'cozo_lib_wasm.js'));
await mod.default({
  module_or_path: new Uint8Array(readFileSync(join(pkg, 'cozo_lib_wasm_bg.wasm'))),
});

const db = mod.CozoDb.new();
const out = [];
for (const [i, script] of steps.entries()) {
  const label = `step${i}`;
  const shown = script.replace(/\s+/g, ' ').slice(0, 100);
  try {
    const res = JSON.parse(db.run(script, '{}', false));
    out.push({
      label,
      script: shown,
      ok: res.ok,
      code: res.code,
      result_keys: Object.keys(res),
      has_took: Object.prototype.hasOwnProperty.call(res, 'took'),
      took: res.took,
      n_rows: Array.isArray(res.rows) ? res.rows.length : undefined,
      rows: Array.isArray(res.rows) ? res.rows.slice(0, 3) : undefined,
      message: res.message ? String(res.message).slice(0, 200) : undefined,
    });
  } catch (e) {
    out.push({ label, script: shown, TRAP: String((e && e.message) || e).slice(0, 300) });
    break;
  }
}
console.log(JSON.stringify(out, null, 1));
