# Live dashboard service — start, stop, logs

The Streamlit live dashboard (`tools/dashboard/app.py`) runs as the systemd service
**`nautilas-dashboard`**: it starts at boot, restarts 5 s after a crash and runs at low
priority (`Nice=10`). It only reads the dashboard Redis, so starting, stopping or crashing it
never affects trading.

Written for nautilas-dashboard.service v1.0.0 / service.sh v1.0.0.

| Item | Where |
|---|---|
| Unit file (source) | `tools/dashboard/nautilas-dashboard.service` |
| Unit file (installed) | `/etc/systemd/system/nautilas-dashboard.service` |
| Control script | `tools/dashboard/service.sh` |
| What the service runs | `tools/dashboard/run.sh` (Streamlit on `127.0.0.1:8501`, user `ubuntu`) |
| Public URL | https://nautilas.awsgoswami.com (Cloudflare → nginx with a password → `127.0.0.1:8501`) |
| Logs | systemd journal (`journalctl -u nautilas-dashboard`) |

App setup, nginx and HTTPS: `tools/dashboard/README.md`. What the bot publishes:
`doc/LIVE_DASHBOARD.md`.

## 1. Install (one-time)

From the repo root (`/home/ubuntu/RustNautilasProject`):

```bash
tools/dashboard/service.sh install
```

It stops a hand-started Streamlit if one holds port 8501, copies the unit to
`/etc/systemd/system/`, enables it at boot, starts it and checks that it answers.
Asks for your sudo password.

Same by hand:

```bash
pkill -f "streamlit run tools/dashboard/app.py"     # only if one was started by hand
sudo cp tools/dashboard/nautilas-dashboard.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now nautilas-dashboard
```

## 2. Daily use

| What | Script | Plain systemd |
|---|---|---|
| Status | `tools/dashboard/service.sh status` | `systemctl status nautilas-dashboard` |
| Start | `tools/dashboard/service.sh start` | `sudo systemctl start nautilas-dashboard` |
| Stop | `tools/dashboard/service.sh stop` | `sudo systemctl stop nautilas-dashboard` |
| Restart (after changing `app.py`) | `tools/dashboard/service.sh restart` | `sudo systemctl restart nautilas-dashboard` |
| Is it answering? | `tools/dashboard/service.sh check` | `curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8501/` (200) |

`stop` lasts until the next `start` or reboot; to keep it off across reboots use
`sudo systemctl disable --now nautilas-dashboard`.

## 3. Logs

| What | Command |
|---|---|
| Follow live (Ctrl+C to quit) | `tools/dashboard/service.sh logs` or `journalctl -u nautilas-dashboard -f` |
| Today | `tools/dashboard/service.sh logs-today` or `journalctl -u nautilas-dashboard --since today` |
| Last 100 lines | `journalctl -u nautilas-dashboard -n 100 --no-pager` |
| A time window | `journalctl -u nautilas-dashboard --since "09:00" --until "10:00"` |
| Since the last boot | `journalctl -u nautilas-dashboard -b` |

These are the dashboard's own logs (Streamlit start-up, Python errors). The trading
program's logs are separate: `logs/wvad-live-YYYY-MM-DD.jsonl`.
nginx: `/var/log/nginx/access.log` and `error.log` (sudo).

## 4. Troubleshooting

| Symptom | Check |
|---|---|
| Site shows 502 | Service down: `service.sh status`, then `service.sh logs-today` for the Python error; `service.sh restart` |
| Service keeps restarting | `journalctl -u nautilas-dashboard -n 50` — usually `config/dashboard.json` missing or Redis down (`redis-cli ping`) |
| "Address already in use" | A hand-started Streamlit holds 8501: `pkill -f "streamlit run tools/dashboard/app.py"`, then `service.sh restart` |
| Page loads but shows no data | The bot is not running or not publishing: check `logs/wvad-live-*.jsonl` and `doc/LIVE_DASHBOARD.md` |
| Changes to `app.py` not visible | `service.sh restart` |
| Unit file changed | `sudo cp tools/dashboard/nautilas-dashboard.service /etc/systemd/system/ && sudo systemctl daemon-reload && service.sh restart` |

## 5. Remove

```bash
tools/dashboard/service.sh uninstall
```

Stops and disables the service and removes the installed unit; `tools/dashboard/run.sh`
still starts it by hand.
