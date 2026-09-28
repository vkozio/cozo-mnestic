/*
 * Copyright 2022, The Cozo Project Authors.
 *
 * This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0.
 * If a copy of the MPL was not distributed with this file,
 * You can obtain one at https://mozilla.org/MPL/2.0/.
 */

// cozo-lib-wasm-next: threaded flavour of the wasm binding.
// Same API as cozo-lib-wasm (CozoDb.new/run/export_relations/import_relations),
// plus init_thread_pool for the rayon worker pool.
//
// Differences from cozo-lib-wasm/src/lib.rs (deliberate, all documented):
// - re-exports wasm_bindgen_rayon::init_thread_pool — JS MUST call it once
//   after init() and before the first query, otherwise rayon runs single-threaded
//   (fallback) or panics on thread::scope paths (see docs, W2/thread::scope).

use wasm_bindgen::prelude::*;

use cozo::*;

mod utils;

// Re-exported so JS gets initThreadPool from the same module:
//   await init();
//   await initThreadPool(navigator.hardwareConcurrency);
pub use wasm_bindgen_rayon::init_thread_pool;

#[wasm_bindgen]
extern "C" {
    fn alert(s: &str);
}

#[wasm_bindgen]
pub struct CozoDb {
    db: DbInstance,
}

#[wasm_bindgen]
impl CozoDb {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        utils::set_panic_hook();
        let db = DbInstance::new("mem", "", "").unwrap();
        Self { db }
    }
    pub fn run(&self, script: &str, params: &str, immutable: bool) -> String {
        self.db.run_script_str(script, params, immutable)
    }
    pub fn export_relations(&self, data: &str) -> String {
        self.db.export_relations_str(data)
    }
    pub fn import_relations(&self, data: &str) -> String {
        self.db.import_relations_str(data)
    }
}
