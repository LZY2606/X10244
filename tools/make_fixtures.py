#!/usr/bin/env python3
"""Generate the offline SAXS case package.

The data are synthetic but shaped like real 2D-integrated SAXS output:
Guinier plateau, a decay consistent with a ~4 nm radius-of-gyration particle
and a flat-ish buffer background. Every file carries units/coordinate basis;
the importer keeps raw strings next to canonical values.
"""
import json
import math
import os

ROOT = os.path.join(os.path.dirname(__file__), "..", "fixtures", "saxs-case")
os.makedirs(ROOT, exist_ok=True)


def particle(q, rg=4.0, i0=60.0):
    # Guinier form at low q; smooth particle decay beyond.
    guinier = i0 * math.exp(-(q * rg) ** 2 / 3.0)
    # pseudo-realistic tail blending toward q^-4 behavior.
    tail = i0 * 220.0 / (1.0 + (q * rg) ** 4)
    w = 1.0 / (1.0 + math.exp((q - 0.35) * 18.0))
    return w * guinier + (1.0 - w) * tail


def buffer_i(q, level=0.8, slope=0.15):
    return level + slope * q + 0.02 * math.sin(50.0 * q)


def write_curve(fname, qs, i_sig, meta_extra=None):
    path = os.path.join(ROOT, fname)
    with open(path, "w") as f:
        f.write("# saxs-bench offline case; columns from 2D azimuthal integration\n")
        f.write("# q[A^-1] I[a.u.] sigmaI[a.u.]\n")
        f.write("# coordinate_basis: reciprocal-space magnitude q=4*pi/lambda*sin(theta)\n")
        for q, (i, s) in zip(qs, i_sig):
            f.write(f"{q:.6f} {i:.6f} {s:.6f}\n")
    if meta_extra:
        with open(path + ".json", "w") as f:
            json.dump(meta_extra, f, indent=2, ensure_ascii=False)


# --- primary case ---------------------------------------------------------
# Integrated q in Angstrom^-1 (raw unit preserved), 0.005..0.50.
qs = [0.005 + 0.495 * k / 59.0 for k in range(60)]

exposure_s, T_s, d_cm = 6.0, 0.82, 0.10
T_b1, T_b2, T_b3 = 0.78, 0.80, 0.70

sample = []
for k, qA in enumerate(qs):
    q_nm = qA * 10.0
    truth = particle(q_nm)
    ib = buffer_i(q_nm)
    raw = (truth + ib) * exposure_s * T_s
    noise = 0.0
    sig = max(0.01 * math.sqrt(max(raw, 0.0)), 0.05)
    sample.append((raw * (1.0 + 0.004 * math.sin(k)), sig))

def buffer_curve(level, slope, exposure, T, seed=0):
    out = []
    for k, qA in enumerate(qs):
        q_nm = qA * 10.0
        raw = buffer_i(q_nm, level, slope) * exposure * T * (1.0 + 0.003 * math.sin(k + seed))
        out.append((raw, max(0.01 * math.sqrt(max(raw, 0.0)), 0.03)))
    return out

b1 = buffer_curve(0.80, 0.15, 6.0, T_b1, 1)
b2 = buffer_curve(0.82, 0.14, 6.0, T_b2, 2)   # near-twin -> multi-candidate ambiguity
b3 = buffer_curve(0.95, 0.40, 6.0, T_b3, 3)   # poor candidate

write_curve("sample_lysozyme_run17.txt", qs, sample, {
    "role": "sample", "name": "lysozyme-run17", "source_file": "sample_lysozyme_run17.txt",
    "exposure": {"value": exposure_s, "unit": "s"},
    "transmission": {"value": T_s, "unit": "-", "sigma": 0.01},
    "thickness": {"value": 1.0, "unit": "mm"},
    "instrument": {"name": "SAXS-beamline-A", "spec": {
        "wavelength_A": 1.033, "detector": "EIGER4M", "geometry": "transmission",
        "coordinate_basis": "q[A^-1]=4*pi/lambda*sin(theta)",
        "response": "per-pixel flat-field + solid-angle applied during integration"}},
    "mask": {"name": "beamstop-run17", "origin": "integration-pipeline mask.fit",
             "locator": "masks/beamstop-run17.fit", "checksum": "sha256:run17-mask"},
})
write_curve("buffer_acetate_v1.txt", qs, b1, {
    "role": "buffer", "name": "acetate-buffer-v1", "source_file": "buffer_acetate_v1.txt",
    "exposure": {"value": 6.0, "unit": "s"},
    "transmission": {"value": T_b1, "unit": "-", "sigma": 0.01},
    "instrument": {"name": "SAXS-beamline-A", "spec": {"wavelength_A": 1.033}},
    "mask": {"name": "beamstop-run17", "origin": "integration-pipeline mask.fit",
             "locator": "masks/beamstop-run17.fit", "checksum": "sha256:run17-mask"}})
write_curve("buffer_acetate_v2.txt", qs, b2, {
    "role": "buffer", "name": "acetate-buffer-v2", "source_file": "buffer_acetate_v2.txt",
    "exposure": {"value": 6.0, "unit": "s"},
    "transmission": {"value": T_b2, "unit": "-", "sigma": 0.01},
    "instrument": {"name": "SAXS-beamline-A", "spec": {"wavelength_A": 1.033}},
    "mask": {"name": "beamstop-run17", "origin": "integration-pipeline mask.fit",
             "locator": "masks/beamstop-run17.fit", "checksum": "sha256:run17-mask"}})
write_curve("buffer_waterbatch.txt", qs, b3, {
    "role": "buffer", "name": "water-batch", "source_file": "buffer_waterbatch.txt",
    "exposure": {"value": 6.0, "unit": "s"},
    "transmission": {"value": T_b3, "unit": "-", "sigma": 0.015},
    "instrument": {"name": "SAXS-beamline-A", "spec": {"wavelength_A": 1.033}},
    "mask": {"name": "beamstop-run17", "origin": "integration-pipeline mask.fit",
             "locator": "masks/beamstop-run17.fit", "checksum": "sha256:run17-mask"}})

# --- boundary fixtures ----------------------------------------------------
# Shifted q grid (0.008..0.48, same span) => interpolation path.
qs_shift = [0.008 + 0.472 * k / 59.0 for k in range(60)]
write_curve("buffer_shifted_grid.txt", qs_shift,
            buffer_curve(0.80, 0.15, 6.0, T_b1, 4), {
    "role": "buffer", "name": "acetate-shifted-grid", "source_file": "buffer_shifted_grid.txt",
    "exposure": {"value": 6.0, "unit": "s"},
    "transmission": {"value": T_b1, "unit": "-", "sigma": 0.01},
    "instrument": {"name": "SAXS-beamline-A", "spec": {"wavelength_A": 1.033}},
    "mask": {"name": "beamstop-run17", "origin": "integration-pipeline mask.fit",
             "locator": "masks/beamstop-run17.fit", "checksum": "sha256:run17-mask"}})

# Disjoint high-q-only buffer => no overlap.
qs_hi = [0.80 + 0.80 * k / 39.0 for k in range(40)]
write_curve("buffer_highq_only.txt", qs_hi,
            [((buffer_i(qA * 10.0) * 6.0 * T_b1), 0.05) for qA in qs_hi], {
    "role": "buffer", "name": "high-q-only", "source_file": "buffer_highq_only.txt",
    "exposure": {"value": 6.0, "unit": "s"},
    "transmission": {"value": T_b1, "unit": "-", "sigma": 0.01},
    "instrument": {"name": "SAXS-beamline-A", "spec": {"wavelength_A": 1.033}},
    "mask": {"name": "beamstop-run17", "origin": "integration-pipeline mask.fit",
             "locator": "masks/beamstop-run17.fit", "checksum": "sha256:run17-mask"}})

# Negative-intensity raw sample (cosmic correction overshoot at low q).
neg = []
for k, (i, s) in enumerate(sample):
    if k in (2, 3):
        neg.append((-abs(i) * 0.02, s))
    else:
        neg.append((i, s))
write_curve("sample_negative_intensity.txt", qs, neg, {
    "role": "sample", "name": "lysozyme-negative-run", "source_file": "sample_negative_intensity.txt",
    "exposure": {"value": 6.0, "unit": "s"},
    "transmission": {"value": T_s, "unit": "-", "sigma": 0.01},
    "thickness": {"value": 1.0, "unit": "mm"},
    "instrument": {"name": "SAXS-beamline-A", "spec": {"wavelength_A": 1.033}},
    "mask": {"name": "beamstop-run17", "origin": "integration-pipeline mask.fit",
             "locator": "masks/beamstop-run17.fit", "checksum": "sha256:run17-mask"}})

# Parameter revision: corrected thickness 1.0mm -> 0.98mm and remeasured T.
with open(os.path.join(ROOT, "sample_lysozyme_run17.revision.json"), "w") as f:
    json.dump({
        "revision_of_source": "sample_lysozyme_run17.txt",
        "name": "lysozyme-run17",
        "thickness": {"value": 0.98, "unit": "mm"},
        "transmission": {"value": 0.83, "unit": "-", "sigma": 0.008},
        "note": "capillary remeasured after beamtime; only metadata changes",
    }, f, indent=2, ensure_ascii=False)

# Local source change: re-integrated last 20 points only (mask tweak), the
# first 40 are byte-identical -> only the high-q dependent region is stale.
local = list(sample)
for k in range(40, 60):
    i, s = local[k]
    local[k] = (i * 1.02, s)
write_curve("sample_lysozyme_run17_reint.txt", qs, local, {
    "role": "sample", "name": "lysozyme-run17", "source_file": "sample_lysozyme_run17_reint.txt",
    "exposure": {"value": 6.0, "unit": "s"},
    "transmission": {"value": 0.83, "unit": "-", "sigma": 0.008},
    "thickness": {"value": 0.98, "unit": "mm"},
    "instrument": {"name": "SAXS-beamline-A", "spec": {"wavelength_A": 1.033}},
    "mask": {"name": "beamstop-run17-mask2", "origin": "re-integration mask v2",
             "locator": "masks/beamstop-run17-v2.fit", "checksum": "sha256:run17-mask-v2"},
    "local_change": {"points_41_60": "re-integrated with mask v2", "points_1_40": "identical"}})

manifest = {
    "case": "散射曲线台：SAXS 归一化、背景扣除与多区间模型证据",
    "coordinate_basis": "q in A^-1 raw / nm^-1 canonical; I in a.u.; t in s; d in mm/cm",
    "materials": [
        "sample_lysozyme_run17.txt", "buffer_acetate_v1.txt", "buffer_acetate_v2.txt",
        "buffer_waterbatch.txt", "buffer_shifted_grid.txt", "buffer_highq_only.txt",
        "sample_negative_intensity.txt", "sample_lysozyme_run17_reint.txt",
        "sample_lysozyme_run17.revision.json"
    ],
    "boundary_situations": [
        "q-grid-misaligned", "no-overlap", "background-scale-uncertainty",
        "negative-intensity", "interval-overlap", "buffer-version-switch",
        "multi-candidate-residual"
    ],
}
with open(os.path.join(ROOT, "manifest.json"), "w") as f:
    json.dump(manifest, f, indent=2, ensure_ascii=False)
print("fixtures written to", ROOT)
