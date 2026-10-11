#!/usr/bin/env bash
# service.sh v1.0.0
# Controls the live dashboard systemd service (nautilas-dashboard, unit file next to this
# script). See doc/DASHBOARD_SERVICE.md.
#   tools/dashboard/service.sh install    # one-time: copy unit, enable at boot, start
#   tools/dashboard/service.sh start|stop|restart|status
#   tools/dashboard/service.sh logs       # follow logs (Ctrl+C to quit)
#   tools/dashboard/service.sh logs-today # today's logs, no follow
#   tools/dashboard/service.sh check      # is Streamlit answering on 127.0.0.1:8501?
#   tools/dashboard/service.sh uninstall  # stop, disable, remove the unit
set -euo pipefail
cd "$(dirname "$0")/../.."
NAME=nautilas-dashboard
UNIT_SRC=tools/dashboard/$NAME.service
UNIT_DST=/etc/systemd/system/$NAME.service
PORT=8501

check() {
  local code
  code=$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$PORT/" || true)
  if [[ "$code" == 200 ]]; then echo "OK: dashboard answering on 127.0.0.1:$PORT"; else echo "NOT OK: 127.0.0.1:$PORT returned '${code:-no answer}'"; return 1; fi
}

case "${1:-}" in
  install)
    if pgrep -f "streamlit run tools/dashboard/app.py" >/dev/null && ! systemctl is-active --quiet "$NAME"; then
      echo "Stopping the hand-started Streamlit (it holds port $PORT)"
      pkill -f "streamlit run tools/dashboard/app.py" || true
      sleep 2
    fi
    sudo cp "$UNIT_SRC" "$UNIT_DST"
    sudo systemctl daemon-reload
    sudo systemctl enable --now "$NAME"
    sleep 5
    systemctl status "$NAME" --no-pager -l | head -12
    check
    ;;
  start|stop|restart)
    sudo systemctl "$1" "$NAME"
    systemctl status "$NAME" --no-pager -l | head -5
    ;;
  status)
    systemctl status "$NAME" --no-pager -l | head -15
    ;;
  logs)
    journalctl -u "$NAME" -f -n 50
    ;;
  logs-today)
    journalctl -u "$NAME" --since today --no-pager
    ;;
  check)
    check
    ;;
  uninstall)
    sudo systemctl disable --now "$NAME" || true
    sudo rm -f "$UNIT_DST"
    sudo systemctl daemon-reload
    echo "Removed $NAME (start by hand again with tools/dashboard/run.sh)"
    ;;
  *)
    sed -n '3,10p' "$0" | sed 's/^# \{0,1\}//'
    exit 1
    ;;
esac
