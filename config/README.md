# Configuration files

| File | Purpose |
|---|---|
| `portfolio-production.json` | Portfolio manifest: the strategy slots (see `doc/PORTFOLIO_SLOTS.md`) |
| `portfolio-development.example.json` | Example manifest for development |
| `wvad-crudeoilm.json` | WVAD settings, the active strategy (see `doc/WVAD_STRATEGY.md`) |
| (`backup/config/`) | Parked Sniper / SATS settings and their portfolio slots (see `backup/README.md`) |
| `kite-production.json` | Broker settings for live runs: Kite user, contract token, MIS, `max_lots`, live-orders gate |
| `kite-production.example.json` | Template for the broker settings |
| `mcx-session-calendar.json` | MCX holidays and special sessions |
| `dashboard.json` | Dashboard Redis URL (`redis_dashboard_url`); gitignored, holds a password (see `doc/LIVE_DASHBOARD.md`) |
| `dashboard.example.json` | Template for the dashboard settings |
