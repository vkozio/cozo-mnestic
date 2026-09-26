# Cozo in web assembly

This crate provides Cozo web assembly modules for browsers.
If you are targeting NodeJS, use [this](../cozo-lib-nodejs) instead: 
native code is still _much_ faster than WASM.

This document describes how to set up the Cozo WASM module for use.
To learn how to use CozoDB (CozoScript), read the [docs](https://docs.cozodb.org/en/latest/index.html).

## Installation

```
npm install https://github.com/vkozio/cozo-mnestic/releases/download/wasm-v0.18.0/cozo-lib-wasm-0.18.0.tar.gz
```

Replace `0.18.0` with the version you want (the artifact name and the release tag both
carry it). This fork publishes to GitHub Releases rather than to the npm registry — the
`cozo-lib-wasm` package on npm is upstream CozoDB, a different database. The installed
package name is still `cozo-lib-wasm`.

Alternatively, you can download `cozo-lib-wasm-<VERSION>.tar.gz`
from the [release page](https://github.com/vkozio/cozo-mnestic/releases) and include
the JS and WASM files directly in your project: see
[`examples/wasm-web-demo`](../examples/wasm-web-demo) for a working Vite setup, and the
`index.html` example
[here](https://rustwasm.github.io/docs/wasm-bindgen/examples/without-a-bundler.html) for
what is required in your code without a bundler at all.

## Usage

> **Read this first:** the CozoScript dialect in this build differs from upstream
> Cozo — sorting is `:sort -age` rather than `| order by age desc`, and a few
> operators are missing or panic outright. Everything verified so far is
> collected in [`docs/wasm-script-notes.md`](docs/wasm-script-notes.md).

See the code [here](../examples/wasm-web-demo/src/main.js). Basically, you write

```js
import init, {CozoDb} from "cozo-lib-wasm";
```

and call

```js
let db;
init().then(() => {
    db = CozoDb.new();
    // db can only be used after the promise resolves 
})
```

## API

```ts
export class CozoDb {
    free(): void;

    static new(): CozoDb;

    run(script: string, params: string, immutable: boolean): string;

    export_relations(data: string): string;

    // Note that triggers are _not_ run for the relations, if any exists.
    // If you need to activate triggers, use queries with parameters.
    import_relations(data: string): string;
}
```

Note that this API is synchronous. If your computation runs for a long time, 
**it will block the main thread**. If you know that some of your queries are going to be heavy,
you should consider running Cozo in a web worker. However, the published module
may not work across browsers in web workers (look for the row "Support for ECMAScript
modules" [here](https://developer.mozilla.org/en-US/docs/Web/API/Worker/Worker#browser_compatibility)).

The next section contains some pointers for how to alleviate this, but expect a lot of work.

## Compiling

You will need to install [Rust](https://rustup.rs/), [NodeJS with npm](https://nodejs.org/),
and [wasm-pack](https://github.com/rustwasm/wasm-pack) first.

The published module was built with

```bash
wasm-pack build --target web --release
```

and the environment variable `CARGO_PROFILE_RELEASE_LTO=fat`.

The important option is `--target web`: the above usage instructions only work for this target.
See the documentation [here](https://rustwasm.github.io/wasm-pack/book/commands/build.html#target).

if you are interested in running Cozo in a web worker and expect it to run across browsers,
you will need to use the `--target no-modules` option, and write a lot of gluing code.
See [here](https://rustwasm.github.io/wasm-bindgen/examples/wasm-in-web-worker.html) for tips.