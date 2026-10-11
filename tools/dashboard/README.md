# Live dashboard (Streamlit)

Read-only web view of the dashboard Redis the bot publishes to (`doc/LIVE_DASHBOARD.md`):
local Redis database 1, as set in `config/dashboard.json`.
Served at https://nautilas.awsgoswami.com (Cloudflare → nginx with a password → Streamlit).

| File | Purpose |
|---|---|
| `app.py` | The Streamlit app |
| `run.sh` | Starts it on `127.0.0.1:8501` (creates `.venv` on first run) |
| `requirements.txt` | Pinned packages |
| `nginx-dashboard.conf` | nginx site for nautilas.awsgoswami.com: password, forwarding to `127.0.0.1:8501` |
| `nautilas-dashboard.service` | systemd unit: runs `run.sh` at boot, restarts on failure |
| `service.sh` | install / start / stop / restart / status / logs for that service |

## Run

In production it runs as the systemd service `nautilas-dashboard`:

```bash
tools/dashboard/service.sh install   # one-time
tools/dashboard/service.sh status    # start | stop | restart | logs | logs-today | check
```

Full guide: `doc/DASHBOARD_SERVICE.md`.

By hand (development, or the demo data):

```bash
tools/dashboard/run.sh                        # kite-prod (the bot)
DASH_PREFIX=kite-demo tools/dashboard/run.sh  # demo data (python3 tools/dashboard/wvad_demo.py: one WVAD slot from a backtest day)
```

Stop the service first (`tools/dashboard/service.sh stop`): both use port 8501.

## nginx with a password (one-time, needs sudo)

Streamlit stays private on 127.0.0.1:8501; nginx serves the site and asks for a password.

```bash
sudo apt-get install -y nginx apache2-utils
sudo htpasswd -c /etc/nginx/dashboard.htpasswd susanta        # choose a password
sudo cp /home/ubuntu/RustNautilasProject/tools/dashboard/nginx-dashboard.conf /etc/nginx/sites-available/dashboard
sudo ln -sf /etc/nginx/sites-available/dashboard /etc/nginx/sites-enabled/dashboard
sudo rm -f /etc/nginx/sites-enabled/default
sudo nginx -t && sudo systemctl reload nginx
```

Open port 80 (TCP) in the Lightsail console: instance → Networking → IPv4 firewall.

## HTTPS on nautilas.awsgoswami.com (Cloudflare in front)

The domain is proxied by Cloudflare. Give nginx a real certificate, then set the Cloudflare
SSL/TLS mode to **Full (strict)** so the Cloudflare → box leg is encrypted and verified too.

```bash
sudo cp /home/ubuntu/RustNautilasProject/tools/dashboard/nginx-dashboard.conf /etc/nginx/sites-available/dashboard
sudo nginx -t && sudo systemctl reload nginx
sudo apt-get install -y certbot python3-certbot-nginx
sudo certbot --nginx -d nautilas.awsgoswami.com --redirect   # adds the 443 block + HTTP -> HTTPS redirect
```

Open port 443 (TCP) in the Lightsail firewall. Certbot renews the certificate automatically.
To stop anyone bypassing Cloudflare via the raw IP, restrict 80/443 in the Lightsail
firewall to Cloudflare's IP ranges (https://www.cloudflare.com/ips/).
