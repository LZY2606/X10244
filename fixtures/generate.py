#!/usr/bin/env python3
"""Deterministic generator for the SAXS case-pack fixtures.

Physics: protein-like Guinier scatterer I0*exp(-q^2 Rg^2/3), I0=1200, Rg=15 nm,
riding on a buffer background b(q) that oscillates at high q (so that the
clipped-loss background-scale scan exhibits multiple local minima).
All noise is a deterministic pseudo-random sequence so fixtures are stable.
"""
import json, math, os

OUT = os.path.dirname(os.path.abspath(__file__))

def noise(i):
    return 0.05 * math.sin(i * 2.7) + 0.03 * math.cos(i * 1.3)

def buffer_level(q):
    base = 0.9
    if q >= 0.30:
        base += 0.25 * math.sin(2 * math.pi * (q - 0.30) / 0.06)
    return base

def protein(q):
    return 1200.0 * math.exp(-75.0 * q * q)

def write_curve(path, rows, q_unit, i_unit, header):
    with open(os.path.join(OUT, path), "w") as f:
        f.write(header + "\n")
        for q, i, s in rows:
            f.write(f"{q:.6f} {i:.6f} {s:.6f}\n")

# --- sample_a: q 0.02 .. 0.612 nm^-1, 75 pts, two negative points at high q
rows = []
for i in range(75):
    q = 0.02 + 0.008 * i
    sig = 0.05 + 0.001 * i
    val = protein(q) + buffer_level(q) + noise(i)
    rows.append([q, val, sig])
rows[72][1] = -0.083   # negative intensity anomaly (kept, flagged)
rows[73][1] = -0.051
write_curve("sample_a.dat", rows, "1/nm", "1/cm",
            "# sample A, lysozyme 8 mg/mL, q(1/nm) I(1/cm) sigma(1/cm)")

# --- buffer_v1: same q grid as sample
rows = []
for i in range(75):
    q = 0.02 + 0.008 * i
    rows.append([q, buffer_level(q) + 0.4 * noise(i + 1000), 0.05])
write_curve("buffer_v1.dat", rows, "1/nm", "1/cm",
            "# buffer, run 41, q(1/nm) I(1/cm) sigma(1/cm)")

# --- buffer_v2: revised buffer measurement (version switch scenario)
rows = []
for i in range(75):
    q = 0.02 + 0.008 * i
    rows.append([q, buffer_level(q) * 1.03 + 0.01 + 0.4 * noise(i + 2000), 0.05])
write_curve("buffer_v2.dat", rows, "1/nm", "1/cm",
            "# buffer, run 42 (revised), q(1/nm) I(1/cm) sigma(1/cm)")

# --- buffer_misaligned: different q grid, partial overlap only
rows = []
for i in range(60):
    q = 0.025 + 0.0075 * i
    rows.append([q, buffer_level(q) + 0.4 * noise(i + 3000), 0.05])
write_curve("buffer_misaligned.dat", rows, "1/nm", "1/cm",
            "# buffer, run 43, alternate q grid, q(1/nm) I(1/cm) sigma(1/cm)")

# --- buffer_nooverlap: disjoint q range
rows = []
for i in range(20):
    q = 0.70 + 0.01 * i
    rows.append([q, 0.9 + 0.4 * noise(i + 4000), 0.05])
write_curve("buffer_nooverlap.dat", rows, "1/nm", "1/cm",
            "# buffer, run 44, high-q only, q(1/nm) I(1/cm) sigma(1/cm)")

meta = {
    "sample": {
        "exposure": {"value": 2.0, "unit": "s"},
        "thickness": {"value": 1.5, "unit": "mm"},
        "transmission": {"value": 0.42, "unit": "1"},
        "exposure_rel_unc": 0.005,
        "thickness_rel_unc": 0.02,
        "transmission_rel_unc": 0.01
    },
    "buffer": {
        "exposure": {"value": 2.0, "unit": "s"},
        "thickness": {"value": 1.5, "unit": "mm"},
        "transmission": {"value": 0.45, "unit": "1"},
        "exposure_rel_unc": 0.005,
        "thickness_rel_unc": 0.02,
        "transmission_rel_unc": 0.01
    },
    "legacy_thickness": {"value": 0.15, "unit": "cm"},
    "capillary_mark": {"value": 3.0, "unit": "marks"}
}
with open(os.path.join(OUT, "meta.json"), "w") as f:
    json.dump(meta, f, indent=2)

instrument = {
    "response": {"kind": "flat_field", "value": 0.98, "unit": "1"},
    "mask_source": "masks/run42_beamstop_v3.png",
    "mask_sha256": "9f2c1ab7",
    "detector": "Eiger2 X 4M",
    "wavelength": {"value": 1.0, "unit": "angstrom"}
}
with open(os.path.join(OUT, "instrument.json"), "w") as f:
    json.dump(instrument, f, indent=2)

print("fixtures written")
