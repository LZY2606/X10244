//! Deterministically generates the offline domain case pack under fixtures/.
//! Only writes repository files; no randomness or network.
use std::fs;
use std::path::Path;

struct Rng(u64);
impl Rng {
    fn next_u(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 11) as f64) / (1u64 << 53) as f64
    }
    fn normal(&mut self) -> f64 {
        let u = (self.next_u().max(1e-12) - 0.5) * 2.0;
        let v = self.next_u();
        (u * std::f64::consts::TAU).cos() * (-2.0 * v.ln()).sqrt()
    }
}

fn qgrid(shift: f64) -> Vec<f64> {
    let n = 40;
    (0..n)
        .map(|i| ((i as f64) * 0.3 / ((n - 1) as f64)) * (1.0 + shift) + 0.010)
        .collect()
}

fn buffer_intensity(i: usize, variant: usize) -> f64 {
    let rg = [18.0, 18.0, 18.0, 18.0, 18.0, 16.0][variant];
    let amp = [2.5e-3, 2.5e-3, 2.5e-3, 2.9e-3, 2.5e-3, 2.5e-3][variant];
    let k = [0.06, 0.06, 0.06, 0.06, 0.05, 0.06][variant];
    let base = 0.0008 * (i + 1) as f64; // smooth trend, not perfectly flat
    amp * (-i as f64 * i as f64 * rg * rg * 0.000020).exp() + k * (i as f64).powi(-3) + base * 1e-4
}

fn write_curve(
    path: &Path,
    q: &[f64],
    i: &[f64],
    sigma: &[f64],
    qunit: &str,
    iunit: &str,
) {
    let mut s = format!("# q[{qunit}],I[{iunit}],sigma[{iunit}]\n");
    for ((&qq, &ii), &ss) in q.iter().zip(i).zip(sigma) {
        s.push_str(&format!("{:.6},{:.8e},{:.8e}\n", qq, ii, ss));
    }
    fs::write(path, s).unwrap();
}

fn write_json(path: &Path, content: &str) {
    fs::write(path, content).unwrap();
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/cases/saxs-bench");
    fs::create_dir_all(&root).unwrap();

    // --- buffers: six variants cover version switch, shift grid, noise levels
    let grids: [Vec<f64>; 6] = [
        qgrid(0.0),
        qgrid(0.0),
        qgrid(0.0),
        qgrid(0.0),
        qgrid(0.012), // shifted grid (misaligned)
        qgrid(0.0),
    ];
    let noise = [0.004, 0.004, 0.004, 0.004, 0.004, 0.025];
    for (v, qv) in grids.iter().enumerate() {
        let mut rng = Rng(700 + v as u64 * 97);
        let mut iv = Vec::new();
        let mut sv = Vec::new();
        for (i, &q) in qv.iter().enumerate() {
            let mut val = buffer_intensity(i, v);
            let sig = (val * noise[v]).max(6e-6);
            val += sig * rng.normal();
            if v == 1 && i >= qv.len() - 3 {
                // low-q drift near the high-q end? actually force a last-bin dip
                val *= 0.86;
            }
            iv.push(val);
            sv.push(sig);
        }
        write_curve(&root.join(format!("buffer_v{v}.csv")), qv, &iv, &sv, "1/A", "1/cm" );
    }
    // buffer metadata: v2 has uncertain scaling, v3 is a separate prep with its own meta
    write_json(
        &root.join("buffer_v0.meta.json"),
        r#"{"logical_id":"buffer_lyso","version":"v0","exposure_s":2.0,"thickness_cm":0.1,"transmission":0.91,"kind":"buffer","note":"first buffer run"}"#,
    );
    write_json(
        &root.join("buffer_v1.meta.json"),
        r#"{"logical_id":"buffer_lyso","version":"v1","exposure_s":2.0,"thickness_cm":0.1,"transmission":0.91,"kind":"buffer","note":"re-run same beamtime, last bins dip"}"#,
    );
    write_json(
        &root.join("buffer_v2.meta.json"),
        r#"{"logical_id":"buffer_lyso","version":"v2","exposure_s":2.0,"thickness_cm":0.1,"transmission":0.93,"scale_uncertainty":0.08,"kind":"buffer","note":"buffer concentration uncertain: scaling error 8%"}"#,
    );
    write_json(
        &root.join("buffer_v3.meta.json"),
        r#"{"logical_id":"buffer_alt","version":"v3","exposure_s":2.0,"thickness_cm":0.1,"transmission":0.88,"kind":"buffer","note":"different buffer prep, different grid intensity"}"#,
    );
    write_json(
        &root.join("buffer_v4.meta.json"),
        r#"{"logical_id":"buffer_shifted","version":"v4","exposure_s":2.0,"thickness_cm":0.1,"transmission":0.91,"kind":"buffer","note":"detector repositioned: shifted q grid"}"#,
    );
    write_json(
        &root.join("buffer_v5.meta.json"),
        r#"{"logical_id":"buffer_noisy","version":"v5","exposure_s":2.0,"thickness_cm":0.1,"transmission":0.91,"kind":"buffer","note":"very noisy buffer: rescaled subtraction gives negatives"}"#,
    );

    // --- samples
    let qs = qgrid(0.0);
    let mut rng = Rng(42);
    let mut sample = Vec::new();
    let mut ssig = Vec::new();
    for (i, &q) in qs.iter().enumerate() {
        let amp = 6.0e-2 * (-q * q * 35.0).exp();
        let porod = 2.2 * q.powi(-4);
        let porod = porod / 1.0e8; // bring to comparable scale at q~0.1
        let plateau = 8.0e-5;
        let buf = buffer_intensity(i, 0) / (2.0 * 0.91) * (0.85 * 0.91); // buffer contribution under sample geometry
        let true_v = amp + porod + plateau + buf;
        let sig = (true_v * 0.006).max(1.5e-5);
        let mut v = true_v + sig * rng.normal();
        if i == 3 {
            v = -3.0e-5; // forced negative intensity outlier
        }
        sample.push(v);
        ssig.push(sig);
    }
    write_curve(&root.join("sample_v0.csv"), &qs, &sample, &ssig, "A^-1", "cm^-1");
    write_json(
        &root.join("sample_v0.meta.json"),
        r#"{"logical_id":"sample_lyso","version":"v0","exposure_s":1.0,"thickness_cm":0.1,"transmission":0.85,"kind":"sample","note":"lysozyme sample, negative outlier bin 3"}"#,
    );
    // revised sample: thickness corrected (local source revision later in replay)
    write_json(
        &root.join("sample_v1.meta.json"),
        r#"{"logical_id":"sample_lyso","version":"v1","exposure_s":1.0,"thickness_cm":0.095,"transmission":0.852,"kind":"sample","note":"thickness remeasured after cell re-alignment"}"#,
    );

    // compact globular protein: broad Guinier window overlapping the Porod window
    let mut rng = Rng(99);
    let mut compact = Vec::new();
    let mut csig = Vec::new();
    for (i, &q) in qs.iter().enumerate() {
        let amp = 4.0e-2 * (-q * q * 9.0).exp(); // Rg ~ 4.2A: decays slowly
        let porod = 0.9 * q.powi(-4) / 1.0e8;
        let buf = buffer_intensity(i, 0) / (2.0 * 0.91) * (0.8 * 0.91);
        let true_v = amp + porod + 6.0e-5 + buf;
        let sig = (true_v * 0.006).max(1.5e-5);
        compact.push(true_v + sig * rng.normal());
        csig.push(sig);
    }
    write_curve(&root.join("sample_compact.csv"), &qs, &compact, &csig, "A^-1", "cm^-1");
    write_json(
        &root.join("sample_compact.meta.json"),
        r#"{"logical_id":"sample_compact","version":"v0","exposure_s":1.0,"thickness_cm":"0.10 cm","transmission":0.8,"kind":"sample","note":"compact particle: broad Guinier window"}"#,
    );

    // metadata carrying an unconvertible value (stays in place with diagnostic)
    write_json(
        &root.join("sample_badmeta.meta.json"),
        r#"{"logical_id":"sample_badmeta","version":"v0","exposure_s":1.0,"thickness_cm":"unknown (calipers broken)","transmission":0.85,"kind":"sample","note":"thickness not machine-convertible"}"#,
    );
    write_curve(&root.join("sample_badmeta.csv"), &qs, &sample, &ssig, "A^-1", "cm^-1");

    // --- instrument response & mask (sources always retained)
    write_json(
        &root.join("instrument_v1.json"),
        r#"{"id":"detector_eiger2","q_basis":"1/A","flat_field":[1.0,1.0,1.0],"note":"flat response for this beamline run"}"#,
    );
    write_json(
        &root.join("mask_v1.json"),
        r#"{"id":"beamstop_mask","excluded_rings":[0,1],"excluded_pixels":4217,"source":"beamline pipeline mask_20260919"}"#,
    );

    // --- manifest: import order and buffer candidate links
    write_json(
        &root.join("manifest.json"),
        r#"{
  "case_id": "saxs-bench",
  "title": "Scattering Curve Bench: SAXS normalization, background subtraction, multi-interval model evidence",
  "instrument": "instrument_v1.json",
  "mask": "mask_v1.json",
  "materials": [
    {"curve": "sample_v0.csv", "meta": "sample_v0.meta.json"},
    {"curve": "sample_v0.csv", "meta": "sample_v1.meta.json"},
    {"curve": "sample_compact.csv", "meta": "sample_compact.meta.json"},
    {"curve": "sample_badmeta.csv", "meta": "sample_badmeta.meta.json"},
    {"curve": "buffer_v0.csv", "meta": "buffer_v0.meta.json"},
    {"curve": "buffer_v1.csv", "meta": "buffer_v1.meta.json"},
    {"curve": "buffer_v2.csv", "meta": "buffer_v2.meta.json"},
    {"curve": "buffer_v3.csv", "meta": "buffer_v3.meta.json"},
    {"curve": "buffer_v4.csv", "meta": "buffer_v4.meta.json"},
    {"curve": "buffer_v5.csv", "meta": "buffer_v5.meta.json"}
  ]
}
"#,
    );

    println!("fixtures written under {}", root.display());
}
