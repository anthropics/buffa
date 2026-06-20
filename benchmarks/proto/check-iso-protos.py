#!/usr/bin/env python3
"""Assert benchmarks/proto/iso/*.proto stay field-identical to the message
blocks in bench_messages.proto. The iso/ files exist so the buffa harness can
compile each message in isolation; bench_messages.proto is the shape source for
every other benchmark consumer (prost, the cross-impl harnesses, dataset
generation). If they drift, buffa would benchmark a different shape than the
implementations it's compared against. Run via `task check-iso-protos`."""
import re, sys, pathlib

here = pathlib.Path(__file__).parent
src = (here / "bench_messages.proto").read_text()

def blocks(text):
    out, i, n = {}, 0, len(text)
    while i < n:
        m = re.compile(r'^message\s+(\w+)\s*\{', re.M).search(text, i)
        if not m: break
        depth, j = 0, m.end() - 1
        while j < n:
            if text[j] == '{': depth += 1
            elif text[j] == '}':
                depth -= 1
                if depth == 0: break
            j += 1
        out[m.group(1)] = re.sub(r'\s+', ' ', text[m.start():j + 1]).strip()
        i = j + 1
    return out

want = blocks(src)
name_to_file = {"ApiResponse": "api_response", "LogRecord": "log_record",
                "AnalyticsEvent": "analytics_event", "MediaFrame": "media_frame",
                "PackedTile": "packed_tile"}
errs = []
for msg, fn in name_to_file.items():
    iso = (here / "iso" / f"{fn}.proto")
    if not iso.exists():
        errs.append(f"missing iso/{fn}.proto"); continue
    got = blocks(iso.read_text()).get(msg)
    if got != want.get(msg):
        errs.append(f"{msg}: iso/{fn}.proto differs from bench_messages.proto")
extra = set(want) - set(name_to_file)
if extra:
    errs.append(f"bench_messages.proto has messages with no iso/ split: {sorted(extra)}")
if errs:
    print("iso proto drift detected:\n  " + "\n  ".join(errs), file=sys.stderr); sys.exit(1)
print(f"ok: {len(name_to_file)} iso protos match bench_messages.proto")
