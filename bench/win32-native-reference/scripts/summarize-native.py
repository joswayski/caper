"""Median (min-max) tables for bench-ui runs including startup (which has no gpu row).
App rows are matched to gpu rows by (variant order, scenario, run) in file order.
Usage: summarize-native.py m.jsonl"""
import json, statistics, sys
from collections import defaultdict

path = sys.argv[1]
app = [json.loads(l) for l in open(path, encoding="utf-8-sig") if l.strip()]
gpu = [json.loads(l) for l in open(path.replace(".jsonl", "-gpu.jsonl"), encoding="utf-8-sig") if l.strip()]
gq = defaultdict(list)
for g in gpu:
    gq[(g["framework"], g["scenario"])].append(g["gpu"])
# app "framework" is the binary's own name; the driver's variant order is native, egui37, cachedgc-styled
name = {"native": "native", "egui37": "egui37", "gpui-styled": "gpui cached+gc"}
gname = {"native": "native", "egui37": "egui37", "gpui-styled": "cachedgc-styled"}
cells = defaultdict(lambda: defaultdict(list))
used = defaultdict(int)
for a in app:
    v, s = a["framework"], a["scenario"].lower()
    g = None
    if s != "startup":
        k = (gname[v], s)
        g = gq[k][used[k]] if used[k] < len(gq[k]) else None
        used[k] += 1
    c = cells[(name[v], s)]
    c["cpu ms/frame"].append(a["cpu_ms_per_frame"])
    c["cpu % core"].append(a["cpu_pct"])
    c["fps"].append(a["fps"])
    c["interval p95 ms"].append(a["frame_p95_ms"])
    c["private MiB"].append(a["pss_mib"])
    c["working set MiB"].append(a["rss_mib"])
    c["first frame ms"].append(a["first_frame_ms"])
    if g:
        c["VRAM MiB"].append(g["dedicated_mib"])
        c["GPU 3D %"].append(g["util_3D"])

def fmt(xs):
    if not xs:
        return "-"
    m = statistics.median(xs)
    d = 2 if m < 10 else 1 if m < 100 else 0
    return f"{m:.{d}f} ({min(xs):.{d}f}-{max(xs):.{d}f})"

variants = ["native", "egui37", "gpui cached+gc"]
scen = ["startup", "idle", "scroll", "burst", "resize", "typing", "hover"]
for metric in ["cpu ms/frame", "cpu % core", "fps", "interval p95 ms", "private MiB", "working set MiB", "VRAM MiB", "GPU 3D %", "first frame ms"]:
    print(f"\n## {metric}\n| scenario | " + " | ".join(variants) + " |\n|---|" + "---:|" * len(variants))
    for s in scen:
        row = [fmt(cells[(v, s)][metric]) for v in variants]
        if any(r != "-" for r in row):
            print(f"| {s} | " + " | ".join(row) + " |")
print("\nruns per cell:", {f"{v}/{s}": len(cells[(v, s)]["fps"]) for v in variants for s in scen})
