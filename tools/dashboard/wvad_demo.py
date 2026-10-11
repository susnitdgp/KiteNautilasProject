#!/usr/bin/env python3
# wvad_demo.py v1.0.0
"""Demo data for the live web dashboard: one WVAD slot built from a real backtest.

Writes the same keys, hash fields, board JSON and event stream the bot writes (see
doc/LIVE_DASHBOARD.md), under the prefix `kite-demo` (never the bot's `kite-prod`), to the
dashboard Redis in config/dashboard.json. No Python packages needed: commands go through
`redis-cli --pipe`.

  python3 tools/dashboard/wvad_demo.py                    # newest WVAD backtest's last day
  python3 tools/dashboard/wvad_demo.py --trades backtest_results/wvad/<symbol>/<run>/trades.csv --day 2026-10-09
  python3 tools/dashboard/wvad_demo.py --clear            # delete the demo keys

The trades (times, levels, fills, exits) are the backtest's; the WVAD / signal chart is an
illustrative wave, not the real indicator values. The last trade is shown still open.
Keys: kite-demo:v1:{crudeoilm-wvad-demo}:dash:state | :events  (expire after 7 days)
"""
import argparse, csv, glob, json, math, os, subprocess, sys, time
from datetime import datetime, timedelta, timezone

IST = timezone(timedelta(hours=5, minutes=30))
TTL = 7 * 86_400
SLOT = "crudeoilm-wvad-demo"


def resp(*args):
    out = [f"*{len(args)}\r\n".encode()]
    for a in args:
        b = a if isinstance(a, bytes) else str(a).encode()
        out.append(f"${len(b)}\r\n".encode() + b + b"\r\n")
    return b"".join(out)


def pipe(url, cmds):
    data = b"".join(resp(*c) for c in cmds)
    r = subprocess.run(["redis-cli", "-u", url, "--no-auth-warning", "--pipe"], input=data, capture_output=True)
    out = r.stdout.decode(errors="replace")
    if r.returncode != 0 or "errors: 0" not in out:
        sys.exit(f"redis-cli --pipe failed: {out.strip()} {r.stderr.decode(errors='replace').strip()}")


def keys(prefix):
    base = f"{prefix}:v1:{{{SLOT}}}:dash"
    return f"{base}:state", f"{base}:events"


def newest_trades_csv():
    runs = sorted(glob.glob("backtest_results/wvad/*/*/trades.csv"), key=os.path.getmtime)
    if not runs:
        sys.exit("No WVAD backtest found: run native-wvad-backtest first, or pass --trades")
    return runs[-1]


def fills(text):
    """'1@8728.4 2@8741' -> [(1, 8728.4), (2, 8741.0)]"""
    return [(int(f.split("@")[0]), float(f.split("@")[1])) for f in text.split()]


def build(rows, day):
    rows = [r for r in rows if r["date"] == day]
    if not rows:
        sys.exit(f"No trades on {day} in that file")
    pv, lots = 10.0, int(rows[0]["lots"])
    trades, events, realized = [], [], 0.0
    t_ms = lambda hhmm, s=0: int(datetime.fromisoformat(f"{day}T{hhmm}:{s:02d}+05:30").timestamp() * 1000)
    for k, r in enumerate(rows):
        d = 1 if r["side"] == "LONG" else -1
        entry, stop, tp1, tp3 = (float(r[c]) for c in ("model_entry", "stop", "tp1", "tp3"))
        risk = abs(entry - stop)
        tps = [tp1, entry + d * 2 * risk, tp3]
        exec_entry = float(r["exec_entry"])
        fl = fills(r["fills"])
        last = k == len(rows) - 1
        hits = [f"TP{i + 1} {tps[i]:,.0f}" for i, (q, p) in enumerate(fl[:-1])]
        pts = sum(q * d * (p - exec_entry) for q, p in fl)
        open_lots = 0.0
        if last:  # show the day's last trade still running: only its partials are booked
            pts = sum(q * d * (p - exec_entry) for q, p in fl[:-1])
            open_lots = float(fl[-1][0])
        realized += pts
        side = "LONG" if d > 0 else "SHORT"
        events.append((t_ms(r["entry_time"]), f"{side} {r['grade']} {lots} lot @ {entry:.0f}  SL {stop:.0f}  TP {tps[0]:.0f}/{tps[1]:.0f}/{tps[2]:.0f}  (target set)"))
        events.append((t_ms(r["entry_time"], 1), f"FILL {'Buy' if d > 0 else 'Sell'} {lots} @ {exec_entry:.0f}"))
        for h in hits:
            events.append((t_ms(r["exit_time"]) - 60_000, f"{h.split()[0]} {h.split()[1]}: close 1 lot"))
        if not last:
            events.append((t_ms(r["exit_time"]), f"EXIT {r['exit_reason']} @ {float(r['model_exit']):.0f}  (model {float(r['model_r']):+.2f} R)"))
        trades.append({
            "opened": f"{r['entry_time']}:00", "dir": d, "lots": lots, "entry": entry, "stop": stop, "tps": tps,
            "grade": r["grade"], "hits": hits,
            "closed": None if last else f"{r['exit_time']}:00",
            "exit": None if last else float(r["model_exit"]),
            "reason": None if last else r["exit_reason"],
            "model_r": None if last else float(r["model_r"]),
            "fill_entry": exec_entry, "open_lots": open_lots, "realized_points": round(pts, 2),
        })
    cur = trades[-1]
    # illustrative WVAD / signal wave over the last 120 bars of the day
    chart = []
    for i in range(120):
        w = 1500 * math.sin(i / 9) + 400 * math.sin(i / 3.3)
        s = 1500 * math.sin((i - 4) / 9) + 150 * math.sin((i - 4) / 3.3)
        minute = 23 * 60 + 15 - (119 - i) * 3
        chart.append([f"{day[-2:]} {minute // 60:02d}:{minute % 60:02d}", [round(w, 1), round(s, 1)]])
    last_price = cur["fill_entry"] + cur["dir"] * 8
    board = {
        "mode": "LIVE (real Zerodha orders)", "slot": SLOT, "instrument": "CRUDEOILM26OCTFUT.MCX",
        "square_off": "23:15", "exit_rule": "TP1 1 / TP2 1 / TP3 1 lot, fixed stop",
        "redis_namespace": f"{day.replace('-', '')}-demo", "point_value": pv, "lots": lots,
        "title": "WVAD v1.1.0 · 3m WVAD 20 · Sma 9 · signal 9 · SL 1.5 ATR14 · ADX14 ≥ 20",
        "model_title": "Williams Variable A/D Pressure",
        "model_rows": [["WVAD 20 / signal 9", "1.6K / 941  (hist 641)"], ["Strength / vol", "36% · vol ×1.0"], ["ADX / HTF", "24 (min 20) · off"]],
        "model_chart_names": ["WVAD", "Signal"], "model_chart": chart, "trades": trades,
        "last_price": last_price, "last_bar": [f"{day[-2:]} Oct 23:12", last_price], "live_bars": 284, "history_bars": 2876,
        "bar_ns": 180_000_000_000, "feed_fault": None, "warmed": True, "trend": cur["dir"],
        "supertrend": None, "tqi": 0.0, "regime": "", "next_r": [0.0, 0.0, 0.0],
        "position": float(cur["dir"]) * cur["open_lots"], "entry_avg": cur["fill_entry"], "sl": cur["stop"],
        "exchange_stop": None, "tps": cur["tps"], "realized_points": round(realized, 2),
        "round_trips": len(trades) - 1, "fills": sum(len(fills(r["fills"])) for r in rows),
        "status": "RUNNING", "halted": None, "dropped": 0, "events": [],
    }
    events.sort()
    return board, events


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--config", default="config/dashboard.json")
    ap.add_argument("--prefix", default="kite-demo")
    ap.add_argument("--trades", help="a WVAD backtest trades.csv (default: the newest)")
    ap.add_argument("--day", help="YYYY-MM-DD (default: the file's last day)")
    ap.add_argument("--clear", action="store_true", help="delete the demo keys and exit")
    a = ap.parse_args()
    if a.prefix == "kite-prod":
        sys.exit("Refusing to write demo data under the bot's own prefix")
    url = json.load(open(a.config))["redis_dashboard_url"]
    state, ev = keys(a.prefix)
    if a.clear:
        pipe(url, [["DEL", state, ev]])
        print("Deleted:", state, ev, sep="\n  ")
        return
    path = a.trades or newest_trades_csv()
    rows = list(csv.DictReader(open(path)))
    day = a.day or rows[-1]["date"]
    board, events = build(rows, day)
    board["events"] = [f"{datetime.fromtimestamp(m / 1000, IST):%H:%M:%S}  {t}" for m, t in reversed(events[-8:])]
    now_ms = int(time.time() * 1000)
    snap = json.dumps({"seq": 1, "updated_at_ms": now_ms, "board": board}, ensure_ascii=False)
    cmds = [["DEL", state, ev],
            ["HSET", state, "snapshot", snap, "updated_at_ms", now_ms, "seq", 1, "status", board["status"]],
            ["EXPIRE", state, TTL]]
    for m, t in events:
        cmds.append(["XADD", ev, "MAXLEN", "~", 1000, f"{m}-*", "ts_ms", m, "text", t])
    cmds.append(["EXPIRE", ev, TTL])
    pipe(url, cmds)
    print(f"{SLOT}: {len(board['trades'])} trades from {path} ({day}), {len(events)} events\n  {state}\n  {ev}")
    print("The snapshot is a one-off: the page marks it stale after 5 s, which is expected for demo data.")


if __name__ == "__main__":
    main()
