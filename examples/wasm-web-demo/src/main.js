// Minimal end-to-end use of the cozo-lib-wasm package published on GitHub
// Releases (not on the npm registry — `cozo-lib-wasm` on npm is upstream CozoDB).
//
//   1. default-import `init` and await it before touching anything else
//   2. create a database instance
//   3. run CozoScript; both results and errors come back as JSON strings
import init, { CozoDb } from 'cozo-lib-wasm';

const statusEl = document.getElementById('status');
const queryEl = document.getElementById('query');
const runEl = document.getElementById('run');
const outEl = document.getElementById('out');
const metaEl = document.getElementById('meta');

async function main() {
  // `init()` fetches cozo_lib_wasm_bg.wasm next to the module and compiles it.
  // Nothing may touch the module before this promise resolves.
  await init();

  const db = CozoDb.new();

  // DDL and DML go in separate run() calls: combining `:create` and `:put` in a
  // single script fails with query::relation_not_found.
  db.run(':create person {name: String => age: Int, city: String}', '{}', false);
  db.run(
    `?[name, age, city] <- [
       ["Ada", 36, "London"],
       ["Grace", 45, "New York"],
       ["Alan", 41, "Cambridge"],
       ["Edsger", 42, "Austin"]
     ] :put person {name, age, city}`,
    '{}',
    false
  );

  statusEl.className = 'ready';
  statusEl.textContent = 'ready — wasm loaded, database seeded';
  runEl.disabled = false;

  const run = () => {
    const t0 = performance.now();
    // immutable = true: read-only, so the engine can skip the write lock
    const res = JSON.parse(db.run(queryEl.value, '{}', true));
    const ms = (performance.now() - t0).toFixed(1);

    if (res.ok) {
      outEl.className = '';
      outEl.textContent = JSON.stringify(res, null, 2);
      metaEl.textContent = `${res.rows.length} row(s) · ${ms} ms · ${res.headers.join(', ')}`;
    } else {
      outEl.className = 'err';
      outEl.textContent = `${res.code ?? 'error'}\n${res.message ?? ''}`;
      metaEl.textContent = `${ms} ms`;
    }
  };

  runEl.addEventListener('click', run);
  run();
}

main().catch((err) => {
  statusEl.className = 'error';
  statusEl.textContent = `failed to start: ${err.message}`;
  outEl.className = 'err';
  outEl.textContent = String(err);
});
