# Следующая сессия — `cozo-lib-wasm-next`

> **Статус: выполнено.** Шаги 1–6 закрыты в сессии 6, плюс сессия 7
> (вендоринг графов — см. `WORKLOG.md` сессии 6–7, `README.md`,
> `HANDOFF.md` §7). П.30 «не чинить здесь» отменён пользователем в сессии 7.
> Открыто: браузерный прогон пула, `opt-level s/z` (R2c), пробы в CI.

## Цель
Довести threaded WASM-билд до проверяемого в браузере состояния: демо с пулом потоков + портированные пробы. Stable-корень и `cozo-lib-wasm` не трогать.

## Стартовое состояние (факт, 2026-09-27)
- Каркас готов: `Cargo.toml` (таблица wasm32-зависимостей слита), `rust-toolchain.toml`, `.cargo/config.toml`, `src/lib.rs` + `src/utils.rs`, `tests/probe/.gitkeep`, `Cargo.lock` (219 пакетов).
- Крейт в `exclude` корневого `Cargo.toml`. Stable `cargo check` в корне зелёный (проверен под sccache и под kache).
- Пайплайн без wasm-pack: `scripts/build.ps1` (cargo → wasm-bindgen → wasm-opt → twiggy). Релиз собран: raw 7 369 652 → bindgen 6 560 236 → wasm-opt `-O3` **6 214 264 байт**. `initThreadPool` в JS-клее есть.
- Доки актуальны: `README.md`, `WORKLOG.md` (сессии 1–5), `HANDOFF.md` (все 7 критериев ✅), `docs/plans/wasm-clean-restart.md` §3.3 согласован с §5.

## Окружение (не менять без причины)
- Тулчейн: `nightly-2026-09-25` + `rust-src` + `wasm32-unknown-unknown` (пин в `rust-toolchain.toml`; внутри крейта plain `cargo` = nightly).
- Кэш: `RUSTC_WRAPPER=kache` (профиль PowerShell; `sccache.exe` лежит как `sccache-0.18.0.bak`, сервер погашен). sccache 0.18.0 этот крейт собрать не может (команда web-sys 50 105 симв. против лимита 32 767 → os error 206); `build.ps1` под sccache отказывается работать — это намеренно.
- Тулы крейтовые: `.tools/bin/wasm-bindgen.exe` 0.2.129 (= версии в `Cargo.lock`, mismatch роняет bindgen), `.tools/bin/wasm-opt.exe` 133. Бутстрап: `scripts/fetch-tools.ps1`.
- Две поправки к гайду уже применены: `--export=__wasm_init_memory` убран (такого символа нет в std этого nightly); `wasm-opt` всегда с `--enable-nontrapping-float-to-int` (иначе валидатор падает на `i64.trunc_sat_f64_s`).
- Нюанс шеллов: процессы, запущенные до переключения профиля, наследуют старый sccache. В начале сессии проверь `$env:RUSTC_WRAPPER` — должен быть `kache`.

## Что делать (по порядку)
1. **Перепроверка.** Новый шелл → `$env:RUSTC_WRAPPER`, `rustc --version` (nightly) внутри крейта, `.\scripts\build.ps1 -Configuration Release`. Ожидание: exit 0, `pkg/` ~6.2 MB.
2. **COOP/COEP демо-харнесс.** Статик-сервер с `Cross-Origin-Opener-Policy: same-origin` + `Cross-Origin-Embedder-Policy: require-corp` (без них `SharedArrayBuffer` и пул не заведутся). Сценарий: `init()` → `initThreadPool(navigator.hardwareConcurrency)` → `CozoDb.new()` → один запрос.
3. **Пробы.** Портировать 21 пробу из `cozo-lib-wasm/tests/probe/probe.mjs` в `tests/probe/` этого крейта, добавив `initThreadPool` до первого кейса. Только чтение `cozo-lib-wasm/tests/probe/probe.mjs` как референса — сам старый крейт не менять.
4. **CI-джоба nightly** только для этого крейта (fetch-tools → build.ps1 Release). Никогда не блокировать stable.
5. **Замеры.** gzip/brotli от `pkg/*_bg.wasm` (CLI на машине нет — жать на сервере или поставить тулы); зафиксировать цифры в WORKLOG. Опционально: сравнение `opt-level s/z` (R2c из плана).
6. **Доки.** Каждый шаг — запись в `WORKLOG.md`; расхождения с фактом править в `README.md`/`HANDOFF.md` сразу.

## Что не делать
- Не запускать nightly-команды из корня; nightly-флаги не должны утекать за пределы крейта.
- Не возвращать wasm-pack и `panic = "abort"` (в `cozo-core` есть `catch_unwind`, план R2c).
- Не чинить `Instant`/`SystemTime` и `page_rank/std::thread::scope` здесь — это волны R1 и вендоринг `graph`, не этот крейт.
- Рабочее дерево грязное: правки в `cozo-lib-wasm/docs/*`, `cozo-lib-wasm/tests/probe/`, `.gortex.yaml`, `AGENTS.md` — чужие, не откатывать и не коммитить без просьбы.

## Быстрые команды
```powershell
$env:RUSTC_WRAPPER                                  # kache?
cd D:\prj-deps\cozo-mnestic\cozo-lib-wasm-next
rustc --version                                     # nightly
.\scripts\build.ps1 -Configuration Release
git status --short                                  # только свои пути
```
