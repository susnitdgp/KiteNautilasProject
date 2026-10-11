# Parked strategies: Precision Sniper and SATS

Moved here on 2026-10-11 (kite-node 2.28.0) so that **WVAD is the only active strategy**.
Nothing in this folder is built, tested or run: the workspace excludes `backup/`
(`exclude = ["backup"]` in the root `Cargo.toml`).

The last state in which both strategies were built, tested and runnable is the git branch
**`backup-2026-10-11-sniper-sats-wvad`** (commit 50bddd6, kite-node 2.27.0) on GitHub.
The SATS-only snapshot before WVAD is the tag `sats-backup-2026-10-11` (8a4eb4d).

| Here | Was |
|---|---|
| `crates/sniper/` | `crates/sniper/` (Precision Sniper v2.1.0 model, crate 2.1.0+4) |
| `crates/sats/` | `crates/sats/` (SATS v1.13.1 model, crate 1.13.1+3) |
| `kite-node/sniper_model.rs` | `apps/kite-node/src/native_node/sniper_model.rs` (Sniper hook-up to the shared runner) |
| `kite-node/sats_*.rs` | `apps/kite-node/src/native_node/sats_{backtest,config,live,strategy,trail}.rs` |
| `config/sniper-crudeoilm.json`, `config/sats-crudeoilm.json` | `config/` |
| `config/portfolio-slots.json` | The `crudeoilm-sniper-202610` and `crudeoilm-sats-202610` entries of `config/portfolio-production.json` |
| `deploy/run-sniper-*.sh`, `deploy/run-sats-*.sh` | `deploy/` |
| `doc/SNIPER_STRATEGY.md`, `doc/SATS_STRATEGY.md` | `doc/` |
| `tools/dash_demo.py` | `tools/` (dashboard demo with Sniper / SATS boards) |

## Bringing one back

Simplest: check out the branch `backup-2026-10-11-sniper-sats-wvad`.

By hand, for Sniper (it uses the same shared runner as WVAD):

1. `git mv backup/crates/sniper crates/sniper`, add it to `members` in `Cargo.toml` and
   `sniper = { path = "../../crates/sniper" }` to `apps/kite-node/Cargo.toml`.
2. `git mv backup/kite-node/sniper_model.rs apps/kite-node/src/native_node/`, add
   `mod sniper_model;` to `native_node/mod.rs`. In it, the model types are now `wvad::Bar`
   / `wvad::Event`: implement `BarModel` for `sniper::Engine` by converting `sniper::Event`
   to `wvad::Event` (same fields), or switch both crates back to one shared type.
3. Add the four `native-sniper-*` commands to `native_node/cli.rs`
   (`bt::run::<sniper::Params>`, `live::run::<sniper::Params>`, `live::preflight::<sniper::Params>`).
4. Move the config, scripts, doc and portfolio slot back (the portfolio holds at most 4 slots).

SATS has its own runner: move `crates/sats`, the five `sats_*.rs` files (with their `mod`
lines), the three `native-sats-*` commands and the `sats` branch of `strategy_settings` in
`portfolio.rs` back, as in the branch above.
