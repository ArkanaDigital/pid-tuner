#!/usr/bin/env python3
"""Generate golden JSON for ArduPilot .bin fixtures using pymavlink DFReader
(the reference decoder). Usage: gen_ap_golden.py <file.bin> [<file.bin> ...]
Writes <file>.golden.json next to each input."""
import json, sys, math
from pymavlink import DFReader

ROW_MSGS = ["RATE", "PIDR", "PIDP", "PIDY", "CTUN", "RCOU", "ATT", "ANG", "ARM", "EV", "MODE", "ISBH"]
HEAD = 50

def num(v):
    if isinstance(v, float):
        return None if (math.isnan(v) or math.isinf(v)) else v
    if isinstance(v, (int,)):
        return v
    return None

def main(path):
    r = DFReader.DFReader_binary(path)
    counts, rows, imu, isbh, parm, msgs = {}, {}, {}, [], {}, []
    first = last = None
    isbd_by_n = {}
    while True:
        m = r.recv_msg()
        if m is None:
            break
        t = m.get_type()
        counts[t] = counts.get(t, 0) + 1
        d = m.to_dict()
        ts = d.get("TimeUS")
        if isinstance(ts, int):
            first = ts if first is None else min(first, ts)
            last = ts if last is None else max(last, ts)
        if t in ROW_MSGS:
            row = {k: num(v) for k, v in d.items() if k != "mavpackettype" and num(v) is not None}
            rows.setdefault(t, {"head": [], "all_tail": []})
            if len(rows[t]["head"]) < HEAD:
                rows[t]["head"].append(row)
            rows[t]["all_tail"].append(row)
            if len(rows[t]["all_tail"]) > HEAD:
                rows[t]["all_tail"].pop(0)
        if t == "IMU":
            inst = d.get("I", 0)
            e = imu.setdefault(str(inst), {"count": 0, "head": []})
            e["count"] += 1
            if len(e["head"]) < HEAD:
                e["head"].append({k: num(d[k]) for k in ("TimeUS", "GyrX", "GyrY", "GyrZ", "GHz") if k in d})
        if t == "ISBH" and len(isbh) < 3:
            isbh.append({k: num(v) for k, v in d.items() if k != "mavpackettype"})
        if t == "ISBD":
            n = d["N"]
            if any(h["N"] == n for h in isbh):
                lst = isbd_by_n.setdefault(n, [])
                if len(lst) < 2:
                    lst.append({"seqno": d["seqno"], "x": list(m.x), "y": list(m.y), "z": list(m.z)})
        if t == "PARM":
            parm[d["Name"]] = d["Value"]
        if t == "MSG":
            msgs.append(d["Message"])
    for k in rows:
        rows[k]["tail"] = rows[k].pop("all_tail")
    fmt = {}
    for mid, f in r.formats.items():
        fmt[f.name] = {"id": mid, "len": f.len, "format": f.format, "columns": list(f.columns)}
    out = {
        "file": path.split("/")[-1],
        "fmt": fmt,
        "counts": counts,
        "time_us": {"first": first, "last": last},
        "rows": rows,
        "imu": imu,
        "isbh": isbh,
        "isbd": {str(k): v for k, v in isbd_by_n.items()},
        "parm": parm,
        "msg": msgs[:10],
    }
    dst = path.rsplit(".", 1)[0] + ".golden.json"
    with open(dst, "w") as f:
        json.dump(out, f, separators=(",", ":"))
    print(dst, "msgs", sum(counts.values()), "fmt", len(fmt), "isbh", len(isbh), "parm", len(parm))

if __name__ == "__main__":
    for p in sys.argv[1:]:
        main(p)
