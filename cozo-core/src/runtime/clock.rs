/*
 * Platform monotonic clock for query budgets (mnestic fork, R1b).
 *
 * `std::time::Instant::now()` panics at runtime on wasm32-unknown-unknown, so
 * every budget/deadline site goes through this alias instead of naming `std`
 * directly: on the host it IS `std` (zero behaviour change), on wasm32 it is
 * `web_time::Instant` (performance.now() with a Date.now() fallback,
 * API-compatible: now/elapsed/checked_add/saturating_duration_since).
 * Wall-clock reads (`SystemTime`) are a separate concern and stay where they
 * are (already Date-gated per site).
 */

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use std::time::Instant;

#[cfg(target_arch = "wasm32")]
pub(crate) use web_time::Instant;
