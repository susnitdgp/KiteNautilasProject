# Project requirements

- No persistent trading state: order records, strategy state and the Nautilus cache live in memory for one run; Kite is the source of truth and there is no order journal or crash recovery. Redis holds only shared/operational data (Kite access token, shared order-rate budget, dashboard). Do not introduce SQLite or another state database.
- One process per slot and account, enforced by an OS lock file (`coordination::lock_instance`).
- Keep each component in a separate module.
- Keep live order submission disabled unless explicitly authorized by user

# Live operation rules (from 12 Oct 2026)

1. **No changes to a live bot during market hours.** While a live run is up (MCX session, 09:00–23:30 IST), do not restart it, edit its config, or run `cargo build`/`cargo test` on the box. Make changes after the close, test them, and start fresh the next morning. The only exception is an emergency stop.
2. **The user runs the bot.** Only the user starts, stops or restarts the live bot (tmux session `bot-1`, `deploy/run-wvad-live.sh`). Agents read logs, check the account and diagnose; they touch `bot-1` only when the user asks for that specific action.
3. **Every stop must explain itself.** Any halt, fault, rejected signal or skipped trade must leave its cause in the run's JSON log (`logs/wvad-live-<date>.jsonl`), e.g. `kite_read_failed` with path, cause and elapsed time. A stop that only says "temporarily unavailable" is a bug to fix.
4. **End-of-day check after every live session:** bot P&L vs Zerodha P&L, every stop and its logged cause, and every bot vs TradingView signal mismatch, before any change for the next day.
