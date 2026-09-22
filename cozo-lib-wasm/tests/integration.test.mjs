import { test } from 'node:test';
import assert from 'node:assert';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const __dirname = dirname(fileURLToPath(import.meta.url));
const pkgPath = join(__dirname, '..', 'pkg');

// Import the compiled WASM module using require for better Windows compatibility
const wasmModule = require(join(pkgPath, 'cozo_lib_wasm.js'));

// Initialize WASM with explicit WASM file path for Node.js
const wasmPath = join(pkgPath, 'cozo_lib_wasm_bg.wasm');
const wasmBuffer = readFileSync(wasmPath);
const wasmModuleBytes = new Uint8Array(wasmBuffer);

await wasmModule.default({ module_or_path: wasmModuleBytes });
const CozoDb = wasmModule.CozoDb;

test('CozoDb.new creates database instance', () => {
  const db = CozoDb.new();
  assert.ok(db !== null);
  assert.ok(typeof db.run === 'function');
  assert.ok(typeof db.export_relations === 'function');
  assert.ok(typeof db.import_relations === 'function');
});

test('run basic query', () => {
  const db = CozoDb.new();
  const result = db.run('?[a] := a = 1', '{}', false);
  assert.ok(typeof result === 'string');
  
  // Verify it's valid JSON with expected structure
  const parsed = JSON.parse(result);
  assert.ok(typeof parsed === 'object');
  assert.strictEqual(parsed.ok, true);
  assert.ok(Array.isArray(parsed.rows));
  assert.strictEqual(parsed.rows.length, 1);
  assert.deepStrictEqual(parsed.rows[0], [1]);
});

test('run query with parameters', () => {
  const db = CozoDb.new();
  const result = db.run('?[a] := a = $x', '{"x": 42}', false);
  assert.ok(typeof result === 'string');
  
  const parsed = JSON.parse(result);
  assert.ok(typeof parsed === 'object');
  assert.strictEqual(parsed.ok, true);
  assert.ok(Array.isArray(parsed.rows));
  assert.strictEqual(parsed.rows.length, 1);
  assert.deepStrictEqual(parsed.rows[0], [42]);
});

test('export_relations returns JSON', () => {
  const db = CozoDb.new();
  // First create some data
  db.run('?[a] := a = 1', '{}', false);
  
  const exported = db.export_relations('[]');
  assert.ok(typeof exported === 'string');
  
  // Verify it's valid JSON
  const parsed = JSON.parse(exported);
  assert.ok(typeof parsed === 'object');
});

test('import_relations processes exported data', () => {
  const db1 = CozoDb.new();
  // Create data in first DB
  db1.run('?[a] := a = 1', '{}', false);
  const exported = db1.export_relations('[]');
  
  // Import into second DB
  const db2 = CozoDb.new();
  const importResult = db2.import_relations(exported);
  assert.ok(typeof importResult === 'string');
  
  // Verify import was successful
  const parsed = JSON.parse(importResult);
  assert.ok(typeof parsed === 'object');
});

test('export_import roundtrip preserves data', () => {
  const db1 = CozoDb.new();
  // Create relation with data
  db1.run('?[a, b] := a = 1, b = 2', '{}', false);
  const exported = db1.export_relations('[]');
  
  // Import into new DB
  const db2 = CozoDb.new();
  db2.import_relations(exported);
  
  // Query both and compare
  const result1 = db1.run('?[a, b] := *relation1[a, b]', '{}', false);
  const result2 = db2.run('?[a, b] := *relation1[a, b]', '{}', false);
  
  assert.strictEqual(result1, result2);
});

test('run with immutable flag', () => {
  const db = CozoDb.new();
  const result = db.run('?[a] := a = 1', '{}', true);
  assert.ok(typeof result === 'string');
  
  const parsed = JSON.parse(result);
  assert.ok(typeof parsed === 'object');
  assert.strictEqual(parsed.ok, true);
  assert.ok(Array.isArray(parsed.rows));
});
