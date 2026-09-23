#!/usr/bin/env python3
"""Deterministic SAXS-bench fixture generator.

Synthesizes 2D-integrated I(q) curves for a protein sample and three buffer
candidate versions.  Values are written with their original units/text
representation intact; the application normalizes only in the derived layer.
All output is deterministic (no timestamps, fixed RNG seed).
"""
import json
import math
import os
import random

HERE = os.path.dirname(os.path.abspath(__file__))
RAW = os.path.join(HERE, "saxs-bench", "raw")
RESP = os.path.join(HERE, "saxs-bench", "response")
MASK = os.path.join(HERE, "saxs-bench", "masks")
for d in (RAW, RESP, MASK):
    os.makedirs(d, exist_ok=True)

random.seed(20260923)

# ---------------------------------------------------------------------------
# q grids
# ---------------------------------------------------------------------------
# A shared aligned grid (sample and buffer A are on the same detector mask)
N = 64
Q_LO, Q_HI = 0.05, 3.2
q_common = [Q_LO * (Q_HI / Q_LO) ** (i / (N - 1)) for i in range(N)]

# A deliberately shifted/denser buffer grid (same physical unit nm^-1 but no
# common q values) -> the "q-grid mismatch / overlap-only interpolation" case
q_shift = [0.062 * (2.9 / 0.062) ** (i / 71.0) for i in range(72)]

# A grid with no physical overlap at low q and truncated before the sample tail
# -> the "no overlap region" edge case when used directly
q_no_overlap = [3.4 * (6.0 / 3.4) ** (i / 23.0) for i in range(24)]


def guinier_porod(q, i0, rg, c4, noise=0.0):
    """Intensity = Guinier plateau + Porod q^-4 tail (physical intensity)."""
    val = i0 * math.exp(-(q * rg) ** 2 / 3.0) + c4 / q ** 4
    if noise:
        val *= 1.0 + random.uniform(-noise, noise)
    return max(val, 1e-9)


def write_curve(path, qs, intensity_counts, error_counts, header_lines):
    with open(path, "w") as fh:
        for line in header_lines:
            fh.write("# " + line + "\n")
        fh.write("q_original,q_unit,I_original,I_unit,error_original,mask_flag\n")
        for q, inten, err in zip(qs, intensity_counts, error_counts):
            fh.write(f"{q:.6f},nm^-1,{inten:.4f},counts,{err:.4f},1\n")


# ---------------------------------------------------------------------------
# Sample: lysozyme-like, Rg=3 nm, I0(physical)=120, strong Porod tail
# ---------------------------------------------------------------------------
random.seed(11)
scale = 1000.0  # detector counts per physical intensity (exposure/flux factor)
sample_phys = [guinier_porod(q, 120.0, 3.0, 0.35, noise=0.004) for q in q_common]
# Inject two beamstop-edge artifacts (still retained, flagged by mask later)
sample_counts = [p * scale * random.uniform(0.99, 1.01) for p in sample_phys]
err_counts = [math.sqrt(c) + 0.01 * c for c in sample_counts]
write_curve(
    os.path.join(RAW, "sample_LYS_20260918.csv"),
    q_common, sample_counts, err_counts,
    [
        "instrument: SAXS-bench LabCamera v4",
        "integration: 2D radial integration pySAXS-like",
        "exposure_seconds: 120",
        "thickness_mm: 1.50",
        "transmission: 0.872",
        "detector_distance_m: 1.20",
        "beam_center_x: 512; beam_center_y: 510",
    ],
)

# ---------------------------------------------------------------------------
# Buffer candidate A: aligned grid (same mask), version 1
# ---------------------------------------------------------------------------
random.seed(21)
bufA_phys = [guinier_porod(q, 6.0, 2.0, 0.35, noise=0.006) for q in q_common]
bufA_counts = [p * scale * random.uniform(0.99, 1.01) * (120.0 / 120.0) for p in bufA_phys]
bufA_err = [math.sqrt(c) + 0.012 * c for c in bufA_counts]
write_curve(
    os.path.join(RAW, "buffer_TRIS_v1_aligned.csv"),
    q_common, bufA_counts, bufA_err,
    [
        "instrument: SAXS-bench LabCamera v4",
        "integration: 2D radial integration",
        "exposure_seconds: 120",
        "thickness_mm: 1.50",
        "transmission: 0.891",
        "buffer_version: TRIS-150mM v1",
    ],
)

# ---------------------------------------------------------------------------
# Buffer candidate B: shifted grid (v2, re-measured after buffer exchange)
# Scale factor is slightly different -> exercises background scale uncertainty
# ---------------------------------------------------------------------------
random.seed(31)
bufB_phys = [guinier_porod(q, 6.6, 2.1, 0.37, noise=0.005) for q in q_shift]
bufB_counts = [p * scale * 1.08 * random.uniform(0.99, 1.01) for p in bufB_phys]
bufB_err = [math.sqrt(c) + 0.012 * c for c in bufB_counts]
write_curve(
    os.path.join(RAW, "buffer_TRIS_v2_shifted.csv"),
    q_shift, bufB_counts, bufB_err,
    [
        "instrument: SAXS-bench LabCamera v4",
        "integration: 2D radial integration",
        "exposure_seconds: 90",
        "thickness_mm: 1.50",
        "transmission: 0.855",
        "buffer_version: TRIS-150mM v2",
    ],
)

# ---------------------------------------------------------------------------
# Buffer candidate C: no physical overlap with the sample low-q region
# ---------------------------------------------------------------------------
random.seed(41)
bufC_phys = [guinier_porod(q, 6.2, 2.0, 0.30, noise=0.005) for q in q_no_overlap]
bufC_counts = [p * scale * random.uniform(0.99, 1.01) for p in bufC_phys]
bufC_err = [math.sqrt(c) + 0.012 * c for c in bufC_counts]
write_curve(
    os.path.join(RAW, "buffer_TRIS_v3_nooverlap.csv"),
    q_no_overlap, bufC_counts, bufC_err,
    [
        "instrument: SAXS-bench LabCamera v4",
        "integration: 2D radial integration",
        "exposure_seconds: 120",
        "thickness_mm: 1.50",
        "transmission: 0.880",
        "buffer_version: TRIS-150mM v3 high-q only",
    ],
)

# ---------------------------------------------------------------------------
# Instrument response (detector sensitivity) + a deliberately malformed alt
# ---------------------------------------------------------------------------
with open(os.path.join(RESP, "detector_response_r2026.json"), "w") as fh:
    json.dump({
        "response_id": "detector-response-r2026",
        "coordinate_basis": "detector_pixel_512x512",
        "normalization_reference": "uniform_water_2026_03",
        "sensitivity_uniform": 1.0,
        "q_independent": True,
        "notes": "Flat-field corrected; treated as unit response for normalized curves",
    }, fh, indent=2, sort_keys=True)
    fh.write("\n")

with open(os.path.join(RESP, "detector_response_legacy_unconvertible.json"), "w") as fh:
    json.dump({
        "response_id": "detector-response-legacy",
        "coordinate_basis": "unknown_scanner_basis",
        "sensitivity_uniform": "see paper scan 1998",
        "q_independent": None,
        "notes": "Legency paper table; cannot convert to current pixel basis",
    }, fh, indent=2, sort_keys=True)
    fh.write("\n")

# ---------------------------------------------------------------------------
# Masks: standard ring mask + a narrow beamstop mask used to exclude artifacts
# ---------------------------------------------------------------------------
def ring_mask(name, beamstop_q_nm, excluded_intervals):
    return {
        "mask_id": name,
        "coordinate_basis": "q_nm^-1_from_beam_center",
        "beamstop_q_nm^-1": beamstop_q_nm,
        "excluded_q_intervals_nm^-1": excluded_intervals,
        "source": "manual ring mask from beamline log",
    }

with open(os.path.join(MASK, "mask_standard.json"), "w") as fh:
    json.dump(ring_mask("mask-standard", 0.045, []), fh, indent=2, sort_keys=True)
    fh.write("\n")
with open(os.path.join(MASK, "mask_beamstop_tight.json"), "w") as fh:
    json.dump(ring_mask("mask-beamstop-tight", 0.052,
                        [[0.05, 0.061]]), fh, indent=2, sort_keys=True)
    fh.write("\n")

print("fixtures written to", HERE)
