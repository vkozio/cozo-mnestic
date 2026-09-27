# HANDOFF: `cozo-lib-wasm-next` — threaded WASM flavour

**Дата:** 2026-09-27  
**Проект:** `D:\prj-deps\cozo-mnestic`  
**Ветка:** `main`, HEAD `06dba804` — “Consolidate wasm notes and gap report”  
**Remote:** `https://github.com/vkozio/cozo-mnestic.git`  
**Причина хендоффа:** провайдер уронил SSE-поток (`JSON error injected into SSE stream`) после запроса “continue work and write document in cozo-lib-wasm-next/ about this crate, and write worklog”. Новый агент должен продолжить с этого места.

---

## 0. TL;DR для следующего агента

Мы начали делать **отдельный крейт `cozo-lib-wasm-next`** — threaded-флейвор WASM-биндинга. Идея: не мучить старый `cozo-lib-wasm` nightly-флагами, atomics, `build-std` и `wasm-bindgen-rayon`, а вынести это в новый крейт с собственным `rust-toolchain.toml`, `.cargo/config.toml` и `Cargo.lock`.

Уже создан каркас:

```text
cozo-lib-wasm-next/
  Cargo.toml
  rust-toolchain.toml
  .cargo/config.toml
  src/lib.rs
  src/utils.rs
  tests/probe/        (пусто)
```

Но есть **два обязательных бага/недоделки**:
1. В `Cargo.toml` дважды объявлена одна и та же TOML-таблица `[target.'cfg(target_arch = "wasm32")'.dependencies]` — TOML это не переварит. Надо слить в одну.
2. `cozo-lib-wasm-next` ещё не добавлен в `exclude` корневого `Cargo.toml`.

Следующий шаг: починить это, проверить `cargo check --target wasm32-unknown-unknown`, создать `README.md`, `WORKLOG.md`, дописать `HANDOFF.md`, обновить `docs/plans/wasm-clean-restart.md` (§3.3 всё ещё говорит про `pkg-threads/`, а §5 уже говорит про `wasm-next`).

---

## 1. Текущее состояние репозитория

`git status --short` до создания `cozo-lib-wasm-next`:

```text
 M .gitignore
 M cozo-lib-wasm/docs/wasm-build-notes.md
 M cozo-lib-wasm/docs/wasm-gap-report.md
 M cozo-lib-wasm/docs/wasm-script-notes.md
?? .gortex.yaml
?? AGENTS.md
?? cozo-lib-wasm/tests/probe/
?? docs/plans/wasm-clean-restart.md
```

После создания нового крейта должно добавиться:

```text
?? cozo-lib-wasm-next/
```

**Важно:** рабочее дерево грязное, там уже есть пользовательские правки в `cozo-lib-wasm/docs/*` и `tests/probe/`. Не откатывать их, не перезаписывать, не коммитить чужие пути без необходимости. `AGENTS.md` и `.gortex.yaml` — тоже не наши.

---

## 2. Что уже сделано в этой сессии

### 2.1. Анализ и документ `docs/plans/wasm-clean-restart.md`

Создан SPEC-документ `docs/plans/wasm-clean-restart.md` (untracked, ~280+ строк). В нём:

- Проверенный факт: `std::thread` и `std::time` на `wasm32-unknown-unknown` остаются заглушками даже с atomics. Ветка `all(target_family = "wasm", target_feature = "atomics")` в `library/std/src/sys/thread/mod.rs:98-104` берёт только `sleep`, а `Thread`/`available_parallelism` идут из `unsupported` в обеих ветках. `Instant`/`SystemTime` паникуют.
- Атомики дают `Mutex`/`Condvar`/`Once`, но не `std::thread::spawn`.
- `rayon` на stable-wasm уходит в single-thread fallback (проверить по залоченной версии; в `cozo-lib-wasm/Cargo.lock` — `rayon 1.10.0`).
- `page_rank` использует `std::thread::scope` (`cozo-core/src/fixed_rule/algos/pagerank.rs:127-131`) — при вендоринге `graph` это придётся переписывать, atomics не помогают. (✅ переписано в сессии 7, см. §7.)
- Волны:
  - **R0** — feature truth: `cozo-lib-wasm/Cargo.toml` как единственное место, отвечающее за wasm-фичи; механизм под `compact-single-threaded`.
  - **R1** — `cozo-core/src/runtime/clock.rs` как единый шим времени; R1a — рефактор без поведения, R1b — `budget_now() → Some`, R1c — `took`/HNSW.
  - **R2** — threads: R2a доки+toolchain, R2b новый крейт, R2c размер/скорость, R2d инструменты.
  - **R3** — консолидация cfg только после CI-гейта.
- §5 обновлён: `wasm-next` вместо `pkg-threads/`-флейвора внутри старого крейта. Правило: **один крейт — один тулчейн — один `Cargo.lock`**.
- Инвариант: stable-сборка (`cargo check`, host-сьюты, `cozo-lib-wasm`) обязана оставаться зелёной; nightly-сборка `cozo-lib-wasm-next` проверяется только своей джобой и никогда не блокирует stable.

### 2.2. Создан каркас `cozo-lib-wasm-next`

Созданы файлы:

#### `cozo-lib-wasm-next/Cargo.toml`

Ключевое:
- package name: `cozo-lib-wasm-next`, version `0.18.0`, edition 2021, crate-type `["cdylib", "rlib"]`.
- dependencies:
  - `wasm-bindgen = "0.2"`
  - `mnestic = { path = "../cozo-core", default-features = false, features = ["wasm", "graph-algo"] }`
  - `getrandom = { version = "0.2", features = ["js", "wasm-bindgen"] }`
  - `console_error_panic_hook` optional
  - target wasm32: `getrandom_03` (package `getrandom` 0.3, features `wasm_js`), `nanorand` 0.7 with `getrandom`, `wasm-bindgen-rayon = "1.3"`
  - dev-dependencies: `wasm-bindgen-test = "0.3"`
- `[patch.crates-io] page_size = { path = "../cozo-lib-wasm/shim/page_size" }`
- release profile: `opt-level = "s"`, `lto = true`, `codegen-units = 1`, `strip = true`; `panic = "abort"` **не включён** намеренно из-за `catch_unwind` в `cozo-core/src/runtime/graph_projection.rs:2383`.

**Баг:** два блока `[target.'cfg(target_arch = "wasm32")'.dependencies]`. Их надо слить в один.

#### `cozo-lib-wasm-next/rust-toolchain.toml`

```toml
[toolchain]
channel = "nightly-2026-09-25"
components = ["rust-src"]
targets = ["wasm32-unknown-unknown"]
```

**Нюанс:** в системе установлен тулчейн `nightly-x86_64-pc-windows-msvc` (версия `1.100.0-nightly (5ceaf6608 2026-09-25)`), а не `nightly-2026-09-25`. Либо установить датированный тулчейн, либо поменять `channel` на `nightly`. `wasm32-unknown-unknown` на nightly уже добавлен вручную в этой сессии.

#### `cozo-lib-wasm-next/.cargo/config.toml`

```toml
[target.wasm32-unknown-unknown]
rustflags = [
    "-C", "target-feature=+atomics,+bulk-memory,+mutable-globals,+simd128",
    "-C", "link-arg=--shared-memory",
    "-C", "link-arg=--import-memory",
    "-C", "link-arg=--export=__wasm_init_tls",
    "-C", "link-arg=--export=__tls_size",
    "-C", "link-arg=--export=__tls_align",
    "-C", "link-arg=--export=__tls_base",
]

[unstable]
build-std = ["panic_abort", "std"]
```

#### `cozo-lib-wasm-next/src/lib.rs`

Повторяет API `cozo-lib-wasm`:
- `pub use wasm_bindgen_rayon::init_thread_pool;`
- `CozoDb` с `new`, `run`, `export_relations`, `import_relations`.
- `mod utils;`

#### `cozo-lib-wasm-next/src/utils.rs`

`set_panic_hook()` — копия из старого крейта.

---

## 3. Что осталось сделать (немедленные шаги)

### Шаг 1. Починить `Cargo.toml`

Слить два блока target-зависимостей в один. Должно получиться примерно так:

```toml
[target.'cfg(target_arch = "wasm32")'.dependencies]
getrandom_03 = { package = "getrandom", version = "0.3", features = ["wasm_js"] }
nanorand = { version = "0.7", features = ["getrandom"] }
wasm-bindgen-rayon = "1.3"
```

Проверить, что `nanorand` и `getrandom` фичи скопированы 1:1 из рабочего `cozo-lib-wasm/Cargo.toml`. Если там `default-features = false`, не потерять.

### Шаг 2. Добавить `cozo-lib-wasm-next` в корневой `exclude`

В `D:\prj-deps\cozo-mnestic\Cargo.toml`:

```toml
exclude = [
    "cozo-lib-c",
    "cozo-lib-java",
    "cozo-lib-wasm",
    "cozo-lib-wasm-next",
    "cozo-lib-swift",
    "cozo-lib-python",
    "cozo-lib-nodejs",
]
```

### Шаг 3. Решить вопрос с тулчейном

Вариант A — оставить `nightly-2026-09-25` и выполнить:

```powershell
rustup toolchain install nightly-2026-09-25
rustup component add rust-src --toolchain nightly-2026-09-25
rustup target add wasm32-unknown-unknown --toolchain nightly-2026-09-25
```

Вариант B — поменять `channel` на `nightly` и убедиться, что для него стоят `rust-src` и `wasm32-unknown-unknown`.

После этого проверить:

```powershell
cd D:\prj-deps\cozo-mnestic\cozo-lib-wasm-next
rustc --version
cargo --version
```

Должен быть nightly.

### Шаг 4. Первая сборка

```powershell
cd D:\prj-deps\cozo-mnestic\cozo-lib-wasm-next
cargo check --target wasm32-unknown-unknown
```

Ожидаемо: `-Z build-std` подтянет `std`/`panic_abort`, соберёт `wasm32-unknown-unknown`. Если падает на `page_size`, проверить путь шима. Если падает на `getrandom`, проверить фичи.

### Шаг 5. Собрать `wasm-pack` или вручную

Попробовать:

```powershell
wasm-pack build --target web --release
```

Если `wasm-pack` ругается на `wasm-opt`/bulk-memory — добавить в `Cargo.toml`:

```toml
[package.metadata.wasm-pack.profile.release]
wasm-opt = false
```

как сделано в старом `cozo-lib-wasm`. Либо собирать вручную через `wasm-bindgen-cli` + `wasm-opt` с `--enable-threads --enable-bulk-memory --enable-simd`.

### Шаг 6. Создать документацию внутри крейта

Нужно создать:
- `cozo-lib-wasm-next/README.md` — что это, чем отличается от `cozo-lib-wasm`, как собирать, как использовать `initThreadPool`, требования COOP/COEP.
- `cozo-lib-wasm-next/WORKLOG.md` — журнал работ: что сделано, что проверено, какие команды запускались, какие ошибки остались.
- `cozo-lib-wasm-next/HANDOFF.md` — этот документ или его обновлённая версия.

### Шаг 7. Проверить stable-инвариант

Убедиться, что старый пайплайн не сломан:

```powershell
cd D:\prj-deps\cozo-mnestic
cargo check
cargo test -p mnestic --test <...>
```

Хотя бы `cargo check` и те host-сьюты, которые уже были зелёными. Не запускать nightly-команды из корня.

### Шаг 8. Обновить `docs/plans/wasm-clean-restart.md`

В §3.3 R2b всё ещё упоминается `cozo-lib-wasm/pkg-threads/`. Привести в соответствие с §5: отдельный крейт `cozo-lib-wasm-next`, а не флейвор внутри старого.

---

## 4. Известные риски и подводные камни

1. **Дублирующаяся TOML-таблица** в `cozo-lib-wasm-next/Cargo.toml` — сборка не стартует.
2. **`cozo-lib-wasm-next` не в `exclude`** — корневой workspace может попытаться его подхватить и смешать `Cargo.lock`.
3. **Тулчейн-пин `nightly-2026-09-25`** может отсутствовать локально; либо ставить, либо менять на `nightly`.
4. **`wasm-pack` 0.14.0** может тащить старый `wasm-opt` 117, который не понимает bulk-memory. В старом крейте это уже лечили через `wasm-opt = false`.
5. **`wasm-bindgen` версия**: в локе старого крейта `0.2.128`. `wasm-bindgen-rayon 1.3.0` требует `^0.2.99`, так что совместимо. Но CLI `wasm-bindgen` должен совпадать с версией в локе. Проверить `wasm-bindgen --version`.
6. **`panic = "abort"`** пока не включать: в `cozo-core` есть `catch_unwind` в `runtime/graph_projection.rs:2383` (тест commit-fence). Сначала переписать тест, потом менять профиль.
7. **`std::thread::scope` в `page_rank`** остаётся проблемой при вендоринге `graph`. К `wasm-next` это не относится напрямую, но помнить. → ✅ закрыто сессией 7: вендор + wasm-fallback (§7 ниже).
8. **`Instant`/`SystemTime`** на wasm32 всё ещё паникуют. Шим `runtime/clock.rs` — отдельная волна R1, не часть `wasm-next`. → ✅ закрыто сессией 7 иначе: точечные шимы вместо единого `clock.rs` (§7 ниже).
9. **COOP/COEP** обязательны для `SharedArrayBuffer`. Без них `initThreadPool` не заработает или молча уйдёт в single-thread.
10. **`--target web`** — `wasm-bindgen-rayon` нормально работает только с web/no-modules. Для Vite-демо нужен `initThreadPool(navigator.hardwareConcurrency)` до первого запроса.

---

## 5. Команды для быстрой проверки

```powershell
# состояние
cd D:\prj-deps\cozo-mnestic
git status --short
git log --oneline -5

# тулчейны
rustup toolchain list
rustup target list --toolchain nightly --installed
rustup component list --toolchain nightly --installed

# новый крейт
cd D:\prj-deps\cozo-mnestic\cozo-lib-wasm-next
rustc --version
cargo check --target wasm32-unknown-unknown

# если нужен wasm-pack
wasm-pack build --target web --release
```

---

## 6. Worklog (append-only)

### 2026-09-27 — сессия 1 (до падения провайдера)

- Изучена история форка, `git log` по `cozo-lib-wasm`.
- Подтверждено: `std::thread`/`std::time` на `wasm32-unknown-unknown` не работают даже с atomics.
- Создан `docs/plans/wasm-clean-restart.md` (SPEC, волны R0–R3).
- Обновлён §5: threads-линейка выносится в `cozo-lib-wasm-next`, старый `cozo-lib-wasm` замораживается.
- Создан каркас `cozo-lib-wasm-next/`:
  - `Cargo.toml`
  - `rust-toolchain.toml`
  - `.cargo/config.toml`
  - `src/lib.rs`
  - `src/utils.rs`
  - пустые `tests/probe/`
- Добавлен `wasm32-unknown-unknown` target на nightly.
- Проверено, что `wasm-bindgen-rayon 1.3.0` зависит от `wasm-bindgen ^0.2.99`, `rayon ^1.8.1` с `web_spin_lock`, `js-sys ^0.3.70`, `crossbeam-channel ^0.5.9`.
- **Не сделано:** фикс дублирующейся TOML-таблицы, добавление в root exclude, `Cargo.lock`, README, WORKLOG, probe-тесты, демо, CI, проверка сборки.

### 2026-09-27 — сессия 2 (продолжение, хендофф выполнен)

- Проверено окружение: `nightly-2026-09-25` уже установлен, `rust-src` +
  `wasm32-unknown-unknown` на месте (вариант A из §3, ставить ничего не надо).
  Внутри `cozo-lib-wasm-next/` `cargo`/`rustc` резолвятся в nightly
  (`cargo 1.100.0-nightly`, `rustc 1.100.0-nightly`) через `rust-toolchain.toml`.
- Шаг 1 done: слит дублирующийся `[target.'cfg(target_arch = "wasm32")'.dependencies]`
  в один блок (`getrandom_03`, `nanorand`, `wasm-bindgen-rayon`); фичи 1:1 со старым
  крейтом. Плюс добавлен `[package.metadata.wasm-pack.profile.release] wasm-opt = false`
  (та же причина, что в старом крейте: bundled wasm-opt 117 не парсит bulk-memory).
- Шаг 2 done: `cozo-lib-wasm-next` добавлен в `exclude` корневого `Cargo.toml`.
- Шаг 4 done: `cargo check --target wasm32-unknown-unknown` → **зелёный (exit 0)**,
  `Cargo.lock` сгенерирован (219 пакетов; `wasm-bindgen 0.2.129`,
  `wasm-bindgen-rayon 1.3.0`, `rayon-core 1.13.0`). Нюанс: первая попытка упала на
  `web-sys` — глобальный `RUSTC_WRAPPER=sccache` даёт `os error 206` (слишком длинная
  командная строка из `--cfg feature="..."` на Windows). Обход: `$env:RUSTC_WRAPPER=""`.
  Задокументировано в `README.md`; нужен постоянный exemption для крейта/CI.
  Остаточные варнинги — преюществующие, не наши (~286 `bail!(err),` future-compat в
  `cozo-core/src/lib.rs`, один `page_size`, один информационный про `-Ctarget-feature: atomics`).
- Шаг 6 done: созданы `README.md` (что/чем отличается/сборка/`initThreadPool`/COOP-EP/
  sccache-quirk), `WORKLOG.md` (сессии 1–2), `tests/probe/.gitkeep` (каталога не было —
  создан как плейсхолдер; портировать 21 пробу из `cozo-lib-wasm/tests/probe/probe.mjs`
  после `pkg/`).
- Шаг 8 done: `docs/plans/wasm-clean-restart.md` §3.3 R2b переписан —
  `pkg-threads/`-флейвор заменён на крейт `cozo-lib-wasm-next`, в соответствии с §5;
  R2a tool-state обновлён под наблюдённые значения (лок `0.2.129`, CLI `0.2.114` mismatch).
- Шаг 7 (stable-инвариант): `cargo check` в корне — см. ниже, выполнить до закрытия сессии.
- **Не сделано / следующей сессии:** `wasm-pack build --target web --release` (блокер —
  системный `wasm-bindgen` CLI `0.2.114` vs `0.2.129` в локе; обновить CLI парой),
  COOP/COEP демо-харнесс, порт 21 пробы с `initThreadPool`, CI-джоба nightly,
  постоянный sccache-exemption.

---

## 7. Критерии готовности для следующей сессии

Статус на конец сессии 5 (2026-09-27, вечер):

1. ✅ `cargo check --target wasm32-unknown-unknown` зелёный (через kache).
2. ✅ Заменено на ручной пайплайн: `scripts/build.ps1 -Configuration Release`
   даёт `pkg/` (6.21 MB после wasm-opt; wasm-pack выведен из эксплуатации).
3. ✅ `README.md` объясняет отличия, `initThreadPool`, пайплайн, kache.
4. ✅ `WORKLOG.md` — сессии 1–5.
5. ✅ `exclude` в корне.
6. ✅ Stable-корень зелёный (проверен под sccache и под kache).
7. ✅ §3.3 согласован с §5.

### 2026-09-27 — сессии 3–5 (кратко, детали в WORKLOG.md)

- sccache 0.18.0 = latest, лечится только апстримом (50 105 симв. vs 32 767;
  PR #2782 покрывает лишь парсинг argfile). `cargo --config
  build.rustc-wrapper=` env НЕ перебивает (доказано пробой с
  несуществующим враппером) — враппер берётся только из окружения процесса.
- kache v0.26.3 установлен (`cargo install`), триал PASS (web-sys зелёный).
  zccache-триал пропущен (kache прошёл + выигрывает по доверию).
- Профиль переключён на kache, sccache в `sccache-0.18.0.bak`, сервер погашен.
  Уже запущенные процессы (включая эту агент-сессию) до рестарта видят
  старый sccache из наследованного окружения.
- `.cargo/config.toml`: добавленное по гайду `--export=__wasm_init_memory`
  убрано (нет такого символа в std этого nightly); лимиты памяти и
  TLS-экспорты валидны (линковка проходит).
- `scripts/{fetch-tools,build}.ps1` + `.tools/` (bindgen 0.2.129, wasm-opt 133)
  + `.gitignore` крейта. wasm-pack-мета удалена из `Cargo.toml`.
- Релизный `pkg/` собран: 7.37 MB → 6.56 MB → 6.21 MB. Нюанс: wasm-opt
  требует `--enable-nontrapping-float-to-int` (dev без него проходит).
- Следующей сессии: COOP/COEP демо-харнесс, порт 21 пробы с `initThreadPool`,
  CI-джоба nightly, gzip/brotli-замеры (CLI на машине нет).

### 2026-09-27 — сессия 6 (NEXT_SESSION steps 1–6, детали в WORKLOG.md)

- Re-verify: релиз пересобран под kache — байт-в-байт как в сессии 5
  (7 369 652 → 6 560 236 → 6 214 264).
- `demo/` (serve.mjs с COOP/COEP/CORP + index.html: init → initThreadPool →
  new → запрос); заголовки проверены curl, браузерный прогон pending.
- `tests/probe/` портирован (21 кейс + initThreadPool, `run-all.ps1`
  зелёный): 16/21 green, `hnsw_query` step2 — артефакт скрипта (как в
  старом), 5 clock-trap как в замороженном билде. Регрессий threaded-сборки
  — ноль. Нюанс: Node-шим `self` обязан форвардить `crypto`, иначе getrandom
  падает (не билд, шим).
- Замеры `*_bg.wasm`: gzip-9 1 934 817 (31.1%), brotli ≈q11 1 319 693
  (21.2%). `opt-level s/z` сравнение (R2c) открыто.
- `.github/workflows/wasm-next.yml`: windows-latest, пин nightly,
  fetch-tools → build.ps1, path-filter, stable не блокирует. Пробы в CI
  не включены (5 известных трапов до R1).

### 2026-09-27 — сессия 7 (вендоринг графов, детали в WORKLOG.md)

- `vendor/graph` (0.3.1) + `vendor/graph_builder` (0.4.1), MIT, upstream
  dormant. `[patch.crates-io]` только в этом крейте — stable не видит.
- Шимы строго под `cfg(target_arch = "wasm32")`, native бит-в-бит:
  `web-time` вместо `Instant` в логах; три рантайм-`scope` (page_rank,
  triangle_count, edgelist) — тот же chunk-loop, 1× вместо N×.
- cozo-core (shared, тоже только wasm-cfg): `stored_queries::create`
  (`created_at` через `Date`), `hnsw_build_index` (проф-часы выкл).
  Остальное уже было под cfg; governed-путь — host-threads, не трогали.
- Нюанс: `js_sys::global()` предпочитает `self`, поэтому Node-стаб пробы
  должен форвардить и `crypto`, и `performance` (иначе падает сам web-time).
- Итог: **21/21 проб green**, exit 0. `pkg` 6 217 260 (+4 КБ),
  gzip-9 1 935 223, brotli 1 320 526. Stable: `cargo check --workspace`
  exit 0, `cargo test -p mnestic --lib` 379/379.
- Открыто: браузерный прогон пула (демо), `s/z` (R2c), пробы в CI
  (первопричина ушла, но CI без пула — решать отдельно).

---

Если что-то в этом хендоффе расходится с фактическим состоянием файлов — **сначала проверь файлы**, потом правь. Рабочее дерево грязное, там есть пользовательские правки, которые нельзя терять.