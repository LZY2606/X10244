#!/usr/bin/env python3
"""Generate offline SAXS case-pack fixtures (deterministic, stdlib only)."""
import csv, json, math, os

ROOT = os.path.join(os.path.dirname(__file__), "..", "fixtures", "saxs")
os.makedirs(ROOT, exist_ok=True)

def write_curve(name, qs, ivals, errs, iunit="count", qunit="1/A"):
    with open(os.path.join(ROOT, name), "w", newline="") as f:
        w = csv.writer(f)
        w.writerow([f"q[{qunit}]", f"I[{iunit}]", f"sigmaI[{iunit}])"])
        for q, i, s in zip(qs, ivals, errs):
            w.writerow([f"{q:.7g}", f"{i:.7g}", f"{s:.7g}"])

def loggrid(q0, q1, n):
    r = (q1 / q0) ** (1.0 / (n - 1))
    return [q0 * r ** k for k in range(n)]

def model(q, i0=120.0, rg=30.0, kp=2.0e-2, bgconst=0.0):
    return i0 * math.exp(-(q * rg) ** 2 / 3.0) + kp * q ** -4 + bgconst

# Deterministic pseudo-noise (no numpy dependency).
def noise(k, amp):
    return amp * math.sin(0.719 * k + 1.3) * math.sin(0.137 * k + 4.1)

N = 64
qs = loggrid(0.008, 0.30, N)

# Sample: counts, noise ~0.6%
iv = [model(q) * (1 + noise(k, 0.006)) for k, q in enumerate(qs)]
er = [max(0.02 * math.sqrt(max(v, 0.0)), v * 0.004) for v in iv]
write_curve("sample_lz_b.dat", qs, iv, er)

# Buffer v1: flat-ish scatter + weak porod tail
def buf(q, amp=2.1, kp=6.0e-3, bump=0.0, bumpq=0.0, bumpw=0.0):
    v = amp + kp * q ** -4
    if bump:
        v += bump * math.exp(-((q - bumpq) / bumpw) ** 2)
    return v
b1 = [buf(q) * (1 + noise(k + 10, 0.003)) for k, q in enumerate(qs)]
be1 = [max(0.02 * math.sqrt(v), v * 0.003) for v in b1]
write_curve("buffer_tris_v1.dat", qs, b1, be1)

# Buffer v2: new batch (slightly higher concentration) -> version switch
b2 = [buf(q, amp=2.35, kp=6.4e-3) * (1 + noise(k + 22, 0.003)) for k, q in enumerate(qs)]
be2 = [max(0.02 * math.sqrt(v), v * 0.003) for v in b2]
write_curve("buffer_tris_v2.dat", qs, b2, be2)

# Same physical range but shifted / denser grid -> q-grid mismatch (still overlapping)
qs2 = loggrid(0.00805, 0.298, 61)
bm = [buf(q, amp=2.1, kp=6.0e-3) * (1 + noise(k + 31, 0.003)) for k, q in enumerate(qs2)]
write_curve("buffer_shifted_grid.dat", qs2, bm, [max(0.02*math.sqrt(v), v*0.003) for v in bm])

# Disjoint grid -> no overlap region
qs3 = loggrid(1.2, 3.0, 40)
bf = [buf(q, amp=1.0, kp=1.0e-5) for q in qs3]
write_curve("buffer_far_grid.dat", qs3, bf, [max(0.01, v*0.005) for v in bf])

# Buffer with a broad bump and large errors -> scale uncertainty + negative points
qb = loggrid(0.008, 0.30, 64)
bb = [buf(q, amp=2.1, kp=6.0e-3, bump=14.0, bumpq=0.05, bumpw=0.012) * (1 + noise(k+44,0.004))
      for k, q in enumerate(qb)]
# generous stated errors -> background scaling uncertainty dominates
beb = [v * 0.06 for v in bb]
write_curve("buffer_bumpy.dat", qb, bb, beb)

# Instrument response (1D efficiency / calibration) with provenance
with open(os.path.join(ROOT, "instrument_response.json"), "w") as f:
    json.dump({
        "id": "resp_pilatus3_2026q2",
        "source": "beamline-calibration/2026-06-30/water_run14",
        "scale": 1.000, "scale_err": 0.002, "scale_unit": "dimensionless",
        "q_basis": "q[1/A] calibrated against silver behenate 58.38 A",
        "note": "detector flat-field + solid-angle correction after azimuthal integration",
    }, f, indent=2)

# Beamstop / detector mask provenance
with open(os.path.join(ROOT, "mask.json"), "w") as f:
    json.dump({
        "id": "mask_beamstop_ring",
        "source": "beamline-masks/2026-06-30/mask_after_burst.tif",
        "masked_q_ranges_1/A": [[0.074, 0.086]],
        "masked_indices_sample": [k for k, q in enumerate(qs) if 0.074 <= q <= 0.086],
        "note": "dead-module ring; points retained in raw layer, excluded from derivation",
    }, f, indent=2)

# Sample run sheet: exposure, thickness, transmission (all with units / raw text)
with open(os.path.join(ROOT, "sample_sheet.json"), "w") as f:
    json.dump({
        "records": [
            {"curve": "sample_lz_b.dat", "id": "sample_lz_b", "role": "sample",
             "name": "lysozyme 4.2 mg/mL in Tris",
             "exposure": {"raw": "1800", "unit": "s"},
             "thickness": {"raw": "0.100", "unit": "cm"},
             "transmission": {"raw": "0.872", "err": "0.003", "unit": "dimensionless"},
             "instrument_response": "resp_pilatus3_2026q2", "mask": "mask_beamstop_ring"},
            {"curve": "buffer_tris_v1.dat", "id": "buffer_tris_v1", "role": "buffer",
             "name": "Tris buffer batch 2026-09-10",
             "exposure": {"raw": "1800", "unit": "s"},
             "thickness": {"raw": "0.100", "unit": "cm"},
             "transmission": {"raw": "0.915", "err": "0.003", "unit": "dimensionless"},
             "instrument_response": "resp_pilatus3_2026q2", "mask": "mask_beamstop_ring"},
            {"curve": "buffer_tris_v2.dat", "id": "buffer_tris_v2", "role": "buffer",
             "name": "Tris buffer batch 2026-09-17",
             "exposure": {"raw": "1800", "unit": "s"},
             "thickness": {"raw": "0.100", "unit": "cm"},
             "transmission": {"raw": "0.909", "err": "0.004", "unit": "dimensionless"},
             "instrument_response": "resp_pilatus3_2026q2", "mask": "mask_beamstop_ring"},
            {"curve": "buffer_shifted_grid.dat", "id": "buffer_shifted_grid", "role": "buffer",
             "name": "Tris buffer re-integrated on shifted q grid",
             "exposure": {"raw": "1800", "unit": "s"}, "thickness": {"raw": "0.100", "unit": "cm"},
             "transmission": {"raw": "0.915", "err": "0.003", "unit": "dimensionless"},
             "instrument_response": "resp_pilatus3_2026q2", "mask": "mask_beamstop_ring"},
            {"curve": "buffer_far_grid.dat", "id": "buffer_far_grid", "role": "buffer",
             "name": "mislabeled high-q scan (grid does not overlap)",
             "exposure": {"raw": "600", "unit": "s"}, "thickness": {"raw": "0.100", "unit": "cm"},
             "transmission": {"raw": "0.920", "err": "0.003", "unit": "dimensionless"},
             "instrument_response": "resp_pilatus3_2026q2", "mask": "mask_beamstop_ring"},
            {"curve": "buffer_bumpy.dat", "id": "buffer_bumpy", "role": "buffer",
             "name": "buffer with aggregation bump (scale poorly constrained)",
             "exposure": {"raw": "1800", "unit": "s"}, "thickness": {"raw": "0.100", "unit": "cm"},
             "transmission": {"raw": "0.912", "err": "0.009", "unit": "dimensionless"},
             "instrument_response": "resp_pilatus3_2026q2", "mask": "mask_beamstop_ring"},
        ]
    }, f, indent=2)

print("fixtures written to", os.path.normpath(ROOT))
