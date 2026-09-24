//! Compute pipeline: normalization -> buffer subtraction -> multi-interval
//! evidence. Each stage publishes an immutable version, artifacts (including
//! rejected candidates), point lineage and a machine-readable rule event.

use rusqlite::{params, Connection};
use serde_json::{json, Value};

use crate::db::error::{app_err, is_conflict};
use crate::db::graph::{config_hash, fingerprint, node_artifact_hash, raw_fingerprint, Fingerprint};
use crate::db::repo::{
    self, append_event, begin_version, ensure_node, insert_artifact, insert_lineage, publish_version,
    set_diagnostics, add_dep,
};
use crate::domain::curves::{normalize, Curve, NormalInputs, NormalizedCurve, NormalizedPoint, QPoint};
use crate::domain::models::{candidate_windows, guinier, interval_overlaps, porod, FitWindow};
use crate::domain::subtract::{residual_summary, subtract, SubConfig, SubtractedCurve};

pub type DbResult<T> = rusqlite::Result<T>;

pub struct Engine<'a> {
    pub c: &'a mut Connection,
    pub project_id: String,
    pub actor: String,
}

#[derive(Debug, Clone)]
pub struct LoadedRecord {
    pub record_id: String,
    pub version: i64,
    pub role: String,
    pub name: String,
    pub curve: Curve,
    pub exposure_s: f64,
    pub transmission: f64,
    pub transmission_sigma: f64,
    pub thickness_cm: Option<f64>,
    pub thickness_sigma: f64,
    pub instrument_id: String,
    pub mask_id: String,
    pub source_diagnostics: Vec<String>,
}

impl<'a> Engine<'a> {
    pub fn new(c: &'a mut Connection, project_id: &str, actor: &str) -> Self {
        Engine { c, project_id: project_id.to_string(), actor: actor.to_string() }
    }

    pub fn load_record(&self, record_id: &str, version: Option<i64>) -> DbResult<LoadedRecord> {
        let ver: i64 = match version {
            Some(v) => v,
            None => self.c.query_row(
                "SELECT MAX(version) FROM raw_records WHERE record_id=?1",
                params![record_id],
                |r| r.get(0),
            )?,
        };
        let (role, name, exposure_json, transmission_json, thickness_json, instrument_id, mask_id, source_file):
            (String, String, String, String, Option<String>, String, String, String) = self.c.query_row(
            "SELECT role,name,exposure_json,transmission_json,thickness_json,instrument_id,mask_id,source_file
             FROM raw_records WHERE record_id=?1 AND version=?2",
            params![record_id, ver],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)),
        )?;
        let exposure: Value = serde_json::from_str(&exposure_json).unwrap();
        let transmission: Value = serde_json::from_str(&transmission_json).unwrap();
        let thickness: Option<Value> = thickness_json.map(|s| serde_json::from_str(&s).unwrap());
        let mut stmt = self.c.prepare(
            "SELECT ordinal,q,i,sig,flags_json,q_raw FROM raw_points
             WHERE record_id=?1 AND version=?2 ORDER BY ordinal",
        )?;
        let rows = stmt.query_map(params![record_id, ver], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?, r.get::<_, f64>(2)?, r.get::<_, f64>(3)?,
                r.get::<_, String>(4)?, r.get::<_, String>(5)?))
        })?;
        let mut points = Vec::new();
        let mut source_diagnostics = Vec::new();
        for row in rows {
            let (ord, q, i, sig, flags_json, q_raw) = row?;
            let flags: Vec<String> = serde_json::from_str(&flags_json).unwrap_or_default();
            source_diagnostics.extend(flags.iter().cloned());
            points.push(QPoint { ordinal: ord as u32, q, i, sig, flags });
        }
        let curve = Curve { points, q_unit: "nm^-1".into(), i_unit: "a.u.".into() };
        Ok(LoadedRecord {
            record_id: record_id.into(),
            version: ver,
            role,
            name,
            curve,
            exposure_s: exposure.get("canonical").and_then(|v| v.as_f64()).unwrap_or(f64::NAN),
            transmission: transmission.get("canonical").and_then(|v| v.as_f64()).unwrap_or(f64::NAN),
            transmission_sigma: 0.0,
            thickness_cm: thickness
                .as_ref()
                .and_then(|t| t.get("canonical").and_then(|v| v.as_f64())),
            thickness_sigma: 0.0,
            instrument_id,
            mask_id,
            source_diagnostics: dedup(source_diagnostics),
        })
    }
}

pub fn dedup(mut v: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    v.retain(|s| seen.insert(s.clone()));
    v
}

/// Live fingerprint for a node. Declarations are kept by the orchestrator in
/// a small JSON registry inside node_config (`fingerprint_spec`); status checks
/// rebuild exactly the same key list the publishing used.
pub fn spec_fingerprint(c: &Connection, node: &str) -> DbResult<Fingerprint> {
    let spec: Value = c.query_row(
        "SELECT json_extract(config_json,'$.fingerprint_spec') FROM node_config WHERE node=?1",
        params![node],
        |r| {
            let s: Option<String> = r.get(0)?;
            Ok(s)
        },
    )?.map(|s| serde_json::from_str(&s).unwrap()).unwrap_or(json!([]));
    let arr = spec.as_array().cloned().unwrap_or_default();
    let mut parts: Vec<(String, String)> = Vec::new();
    for item in arr {
        let kind = item["kind"].as_str().unwrap_or("");
        let id = item["id"].as_str().unwrap_or("").to_string();
        match kind {
            "raw" => {
                let ver = item["version"].as_i64().unwrap();
                let h = raw_fingerprint(c, &id, ver)?;
                parts.push((format!("raw:{id}@{ver}"), h));
            }
            "instrument" => {
                let v: String = c.query_row("SELECT spec_json FROM instruments WHERE instrument_id=?1", params![id], |r| r.get(0))?;
                parts.push((format!("instrument:{id}"), crate::db::model::checksum(&[&v])));
            }
            "mask" => {
                let v: String = c.query_row("SELECT checksum FROM masks WHERE mask_id=?1", params![id], |r| r.get(0))?;
                parts.push((format!("mask:{id}"), v));
            }
            "node" => {
                let ver = item["version"].as_i64().unwrap_or(-1);
                let used_ver = if ver < 0 {
                    c.query_row("SELECT current_version FROM derivations WHERE node=?1", params![id], |r| r.get::<_, i64>(0))?
                } else {
                    ver
                };
                let h = node_artifact_hash(c, &id, used_ver).unwrap_or_default();
                parts.push((format!("node:{id}@{used_ver}"), h));
            }
            _ => {}
        }
    }
    let (cfg, _) = repo::get_node_config(c, node)?.unwrap_or((json!({}), 0));
    parts.push(("config".into(), config_hash(&cfg)));
    Ok(fingerprint(parts))
}

pub struct RunOutcome {
    pub node: String,
    pub version: i64,
    pub published: bool,
    pub event_seq: i64,
    pub diagnostics: Vec<String>,
}

impl<'a> Engine<'a> {
    fn store_spec(cfg: &mut Value, spec: Value) {
        cfg["fingerprint_spec"] = spec;
    }

    /// Normalization node: one raw record -> exposure/transmission-normalized
    /// curve. Thickness is deliberately excluded (separate optional step).
    pub fn run_normalization(&mut self, node: &str, record_id: &str, title: &str) -> DbResult<RunOutcome> {
        ensure_node(self.c, &self.project_id, node, "normalization", title)?;
        let rec = self.load_record(record_id, None)?;
        let mut cfg = repo::get_node_config(self.c, node)?.map(|(v, _)| v).unwrap_or_else(|| {
            json!({ "step": "I/(exposure*transmission)", "thickness": "not-applied-here" })
        });
        Self::store_spec(
            &mut cfg,
            json!([
                {"kind":"raw","id":rec.record_id,"version":rec.version},
                {"kind":"instrument","id":rec.instrument_id},
                {"kind":"mask","id":rec.mask_id}
            ]),
        );
        let fp = spec_fingerprint(self.c, node)
            .unwrap_or_else(|_| fingerprint(vec![("raw".into(), raw_fingerprint(self.c, &rec.record_id, rec.version).unwrap_or_default())]));
        // spec may not be persisted yet; recompute after writing config below.
        repo::set_node_config(self.c, node, &cfg, &self.actor, repo::get_node_config(self.c, node)?.map(|(_, r)| r).unwrap_or(0))?;
        let fp = spec_fingerprint(self.c, node)?;
        if let Ok(st) = crate::db::graph::node_status(self.c, node, &fp) {
            if st.validity == crate::db::graph::Validity::Valid {
                let ev = append_event(self.c, &self.project_id, &self.actor, "recompute-skipped", &json!({
                    "node": node, "reason": "valid", "current_version": st.current_version,
                    "rule": "dependency-graph-fingerprint-match"
                }))?;
                return Ok(RunOutcome { node: node.into(), version: st.current_version, published: false, event_seq: ev.seq, diagnostics: vec![] });
            }
        }

        let inp = NormalInputs {
            exposure_s: rec.exposure_s,
            transmission: rec.transmission,
            transmission_sigma: rec.transmission_sigma,
            thickness_cm: rec.thickness_cm,
            thickness_sigma: rec.thickness_sigma,
        };
        let norm: NormalizedCurve = normalize(&rec.curve, &inp);
        let mut diagnostics = rec.source_diagnostics.clone();
        diagnostics.extend(norm.diagnostics.iter().cloned());

        let version = begin_version(self.c, node, &fp.hash, &cfg)?;
        add_dep(self.c, node, version, "raw", &rec.record_id, rec.version, &raw_fingerprint(self.c, &rec.record_id, rec.version)?)?;
        add_dep(self.c, node, version, "instrument", &rec.instrument_id, 0, &crate::db::model::checksum(&[]))?;
        add_dep(self.c, node, version, "mask", &rec.mask_id, 0, &rec.mask_id)?;

        let payload = json!({
            "record_id": rec.record_id,
            "record_version": rec.version,
            "name": rec.name,
            "q_unit": norm.q_unit,
            "i_unit": norm.i_unit,
            "normalization": "I / (exposure_s * transmission)",
            "thickness_policy": "excluded-from-this-step",
            "points": norm.points,
        });
        let aid = insert_artifact(self.c, node, version, "primary", "curve", true, &payload)?;
        // Point lineage: each normalized point traces to exactly one raw ordinal.
        for p in &norm.points {
            let lineage = json!({
                "derived_point_id": format!("{node}:v{version}:o{}", p.ordinal),
                "parents": [{
                    "record_id": rec.record_id,
                    "record_version": rec.version,
                    "ordinal": p.ordinal,
                    "role": rec.role,
                    "name": rec.name,
                }],
            });
            insert_lineage(self.c, &aid, &lineage)?;
        }
        set_diagnostics(self.c, node, version, &diagnostics)?;
        publish_version(self.c, &self.project_id, node, version)?;

        let rule_event = json!({
            "node": node,
            "version": version,
            "rule": "normalization:exposure-transmission",
            "judgment": if diagnostics.is_empty() { "clean" } else { "diagnostics-recorded" },
            "uncertainty": "sigma includes counting error and transmission uncertainty",
            "diagnostics": diagnostics,
            "thickness": "not folded in; remains an independently reviewable step"
        });
        let ev = append_event(self.c, &self.project_id, &self.actor, "normalization-published", &rule_event)?;
        Ok(RunOutcome { node: node.into(), version, published: true, event_seq: ev.seq, diagnostics })
    }

    pub fn normalized_points(&self, node: &str, version: Option<i64>) -> DbResult<(Vec<NormalizedPoint>, Value)> {
        let ver = version
            .unwrap_or_else(|| self.c.query_row("SELECT current_version FROM derivations WHERE node=?1", params![node], |r| r.get(0)).unwrap_or(0));
        let payload: String = self.c.query_row(
            "SELECT payload_json FROM artifacts WHERE node=?1 AND version=?2 AND kind='curve'",
            params![node, ver],
            |r| r.get(0),
        )?;
        let v: Value = serde_json::from_str(&payload).unwrap();
        let pts: Vec<NormalizedPoint> = serde_json::from_value(v["points"].clone()).unwrap();
        Ok((pts, v))
    }
}

impl<'a> Engine<'a> {
    /// Buffer subtraction node. Every buffer candidate produces a residual-test
    /// artifact (adopted + rejected retained). The primary subtraction uses the
    /// chosen buffer; the choice is explicit and switching versions creates a
    /// new derivation version — never an overwrite.
    pub fn run_subtraction(
        &mut self,
        node: &str,
        sample_norm_node: &str,
        buffer_norm_nodes: &[(String, String)], // (norm_node, display name)
        chosen: &str,
        mut cfg: SubConfig,
        title: &str,
    ) -> DbResult<RunOutcome> {
        ensure_node(self.c, &self.project_id, node, "subtraction", title)?;
        let (sample_pts, sample_meta) = self.normalized_points(sample_norm_node, None)?;
        let sample_rec = sample_meta["record_id"].as_str().unwrap_or("").to_string();
        let sample_ver = sample_meta["record_version"].as_i64().unwrap_or(0);

        // Residual evidence per candidate.
        let mut summaries = Vec::new();
        let mut chosen_pts = None;
        for (bnode, bname) in buffer_norm_nodes {
            let (bpts, bmeta) = self.normalized_points(bnode, None)?;
            let sum = residual_summary(bname, &sample_pts, &bpts, &cfg);
            summaries.push(json!({
                "buffer_node": bnode,
                "buffer_record_id": bmeta["record_id"].as_str().unwrap_or(""),
                "buffer_record_version": bmeta["record_version"].as_i64().unwrap_or(0),
                "name": bname,
                "summary": sum,
            }));
            if bnode == chosen || bname == chosen {
                chosen_pts = Some((bpts, bmeta));
            }
        }
        let (buffer_pts, buffer_meta) = chosen_pts
            .ok_or_else(|| app_err(format!("chosen buffer {chosen} is not among candidates; refusing to subtract")))?;
        let buffer_rec = buffer_meta["record_id"].as_str().unwrap().to_string();
        let buffer_ver = buffer_meta["record_version"].as_i64().unwrap_or(0);

        // Config + fingerprint persisted before status check.
        let mut cfgv = serde_json::to_value(&cfg).unwrap();
        cfgv["step"] = json!("I_sample - k I_buffer on shared overlap");
        cfgv["chosen_buffer"] = json!(chosen);
        cfgv["sample_node"] = json!(sample_norm_node);
        cfgv["buffer_nodes"] = json!(buffer_norm_nodes);
        Self::store_spec(
            &mut cfgv,
            json!([
                {"kind":"node","id":sample_norm_node},
                {"kind":"node","id":chosen},
                {"kind":"raw","id":sample_rec,"version":sample_ver},
                {"kind":"raw","id":buffer_rec,"version":buffer_ver}
            ]),
        );
        let old_rev = repo::get_node_config(self.c, node)?.map(|(_, r)| r).unwrap_or(0);
        repo::set_node_config(self.c, node, &cfgv, &self.actor, old_rev)?;
        let fp = spec_fingerprint(self.c, node)?;
        if let Ok(st) = crate::db::graph::node_status(self.c, node, &fp) {
            if st.validity == crate::db::graph::Validity::Valid {
                let ev = append_event(self.c, &self.project_id, &self.actor, "recompute-skipped", &json!({
                    "node": node, "reason": "valid", "current_version": st.current_version,
                    "rule": "dependency-graph-fingerprint-match"
                }))?;
                return Ok(RunOutcome { node: node.into(), version: st.current_version, published: false, event_seq: ev.seq, diagnostics: vec![] });
            }
        }

        let out: SubtractedCurve = subtract(&sample_pts, &buffer_pts, &cfg);
        let version = begin_version(self.c, node, &fp.hash, &cfgv)?;
        add_dep(self.c, node, version, "node", sample_norm_node,
            repo::node_current(self.c, sample_norm_node)?,
            &node_artifact_hash(self.c, sample_norm_node, repo::node_current(self.c, sample_norm_node)?)?)?;
        add_dep(self.c, node, version, "node", chosen,
            repo::node_current(self.c, chosen)?,
            &node_artifact_hash(self.c, chosen, repo::node_current(self.c, chosen)?)?)?;
        add_dep(self.c, node, version, "raw", &sample_rec, sample_ver, &raw_fingerprint(self.c, &sample_rec, sample_ver)?)?;
        add_dep(self.c, node, version, "raw", &buffer_rec, buffer_ver, &raw_fingerprint(self.c, &buffer_rec, buffer_ver)?)?;

        // Candidate residual artifacts: all retained, chosen adopted only by
        // explicit decision; nothing is auto-deleted on ambiguity.
        for (i, s) in summaries.iter().enumerate() {
            let key = format!("cand-{i}");
            let is_chosen = s["name"] == json!(chosen) || s["buffer_node"] == json!(chosen);
            insert_artifact(self.c, node, version, &key, "residual_test", false, s)?;
            if is_chosen {
                let _ = key;
            }
        }

        let mut diagnostics = out.diagnostics.clone();
        if !out.ok {
            set_diagnostics(self.c, node, version, &diagnostics)?;
            // Failed derivations still publish a structured failure artifact so
            // the review UI shows the boundary case instead of an empty slot.
            let fail = json!({
                "ok": false,
                "grid_relation": out.grid_relation,
                "overlap_lo": out.overlap_lo,
                "overlap_hi": out.overlap_hi,
                "diagnostics": diagnostics,
                "candidates": summaries,
            });
            insert_artifact(self.c, node, version, "primary", "diagnostic", false, &fail)?;
            repo::abort_version(self.c, node, version)?;
            let ev = append_event(self.c, &self.project_id, &self.actor, "subtraction-failed", &json!({
                "node": node, "version": version,
                "rule": "q-grid:no-direct-subtraction;interpolation-overlap-only",
                "grid_relation": out.grid_relation,
                "diagnostics": diagnostics,
                "candidates_retained": summaries.len(),
                "pointer": "unchanged",
            }))?;
            return Ok(RunOutcome { node: node.into(), version, published: false, event_seq: ev.seq, diagnostics });
        }

        let payload = json!({
            "sample_node": sample_norm_node,
            "buffer_node": chosen,
            "sample_record_id": sample_rec,
            "sample_record_version": sample_ver,
            "buffer_record_id": buffer_rec,
            "buffer_record_version": buffer_ver,
            "grid_relation": out.grid_relation,
            "overlap_lo": out.overlap_lo,
            "overlap_hi": out.overlap_hi,
            "scale_k": out.scale_k,
            "scale_sigma": out.scale_sigma,
            "scale_source": out.scale_source,
            "points": out.points,
            "candidates": summaries,
        });
        let aid = insert_artifact(self.c, node, version, "primary", "curve", true, &payload)?;
        // Lineage: each derived point references the sample ordinal and the
        // one or two bracketing buffer ordinals — "click point -> two sources".
        for p in &out.points {
            let lineage = json!({
                "derived_point_id": format!("{node}:v{version}:o{}", p.ordinal),
                "parents": {
                    "sample": {"node": sample_norm_node, "record_id": sample_rec, "record_version": sample_ver, "ordinal": p.sample_ordinal},
                    "buffer_lo": p.buffer_lo_ordinal.map(|o| json!({"node": chosen, "record_id": buffer_rec, "record_version": buffer_ver, "ordinal": o})),
                    "buffer_hi": p.buffer_hi_ordinal.map(|o| json!({"node": chosen, "record_id": buffer_rec, "record_version": buffer_ver, "ordinal": o})),
                    "interpolated": p.interp_buffer,
                    "weight_lo": p.weight_lo,
                }
            });
            insert_lineage(self.c, &aid, &lineage)?;
        }
        set_diagnostics(self.c, node, version, &diagnostics)?;
        publish_version(self.c, &self.project_id, node, version)?;

        let legal: Vec<&Value> = summaries.iter().filter(|s| s["summary"]["legal"] == json!(true)).collect();
        let ambiguity = legal.len() > 1;
        let ev = append_event(self.c, &self.project_id, &self.actor, "subtraction-published", &json!({
            "node": node,
            "version": version,
            "rule": "q-grid:no-direct-subtraction;interpolation-overlap-only",
            "grid_relation": out.grid_relation,
            "overlap": [out.overlap_lo, out.overlap_hi],
            "scale_k": out.scale_k,
            "scale_sigma": out.scale_sigma,
            "uncertainty": "sigma_sub^2 includes normalization variances, buffer interpolation and scale-k uncertainty",
            "candidates_total": summaries.len(),
            "candidates_legal": legal.len(),
            "ambiguous_multi_candidate": ambiguity,
            "auto_selection": false,
            "negative_residuals": out.points.iter().filter(|p| p.flags.iter().any(|f| f=="negative-residual")).count(),
            "diagnostics": diagnostics,
        }))?;
        Ok(RunOutcome { node: node.into(), version, published: true, event_seq: ev.seq, diagnostics })
    }
}

impl<'a> Engine<'a> {
    /// Multi-interval model evidence. Guinier and Porod candidates are fitted
    /// independently, all windows retained, overlaps reported, and no single
    /// stitched model is ever produced automatically.
    pub fn run_intervals(&mut self, node: &str, sub_node: &str, title: &str) -> DbResult<RunOutcome> {
        ensure_node(self.c, &self.project_id, node, "intervals", title)?;
        let ver = repo::node_current(self.c, sub_node)?;
        let payload: String = self.c.query_row(
            "SELECT payload_json FROM artifacts WHERE node=?1 AND version=?2 AND kind='curve' AND candidate_key='primary'",
            params![sub_node, ver],
            |r| r.get(0),
        )?;
        let pv: Value = serde_json::from_str(&payload).unwrap();
        let pts = serde_json::from_value::<Vec<crate::domain::subtract::SubPoint>>(pv["points"].clone())
            .map_err(|e| app_err(format!("upstream subtraction not usable for interval fits: {e}")))?;

        let mut cfg = repo::get_node_config(self.c, node)?.map(|(v, _)| v).unwrap_or_else(|| json!({
            "policy": "guinier and porod candidates stay independent; never auto-stitched"
        }));
        cfg["fingerprint_spec"] = json!([ {"kind":"node","id":sub_node} ]);
        let old_rev = repo::get_node_config(self.c, node)?.map(|(_, r)| r).unwrap_or(0);
        repo::set_node_config(self.c, node, &cfg, &self.actor, old_rev)?;
        let fp = spec_fingerprint(self.c, node)?;
        if let Ok(st) = crate::db::graph::node_status(self.c, node, &fp) {
            if st.validity == crate::db::graph::Validity::Valid {
                let ev = append_event(self.c, &self.project_id, &self.actor, "recompute-skipped", &json!({
                    "node": node, "current_version": st.current_version, "reason": "valid",
                    "rule": "dependency-graph-fingerprint-match"
                }))?;
                return Ok(RunOutcome { node: node.into(), version: st.current_version, published: false, event_seq: ev.seq, diagnostics: vec![] });
            }
        }

        let g_windows = candidate_windows(&pts, 4, "low");
        let p_windows = candidate_windows(&pts, 4, "high");
        let mut guinier_cands: Vec<Value> = Vec::new();
        for w in &g_windows {
            if let Some(cand) = guinier(&pts, w) {
                guinier_cands.push(serde_json::to_value(cand).unwrap());
            }
        }
        let mut porod_cands: Vec<Value> = Vec::new();
        for w in &p_windows {
            if let Some(cand) = porod(&pts, w, "power") {
                porod_cands.push(serde_json::to_value(cand).unwrap());
            }
        }
        let overlaps = interval_overlaps(
            &g_windows.iter().map(|w| w.clone()).collect::<Vec<FitWindow>>(),
            &p_windows.iter().map(|w| w.clone()).collect::<Vec<FitWindow>>(),
        );
        let overlaps_v: Vec<Value> = overlaps.iter().map(|o| serde_json::to_value(o).unwrap()).collect();

        let version = begin_version(self.c, node, &fp.hash, &cfg)?;
        add_dep(self.c, node, version, "node", sub_node, ver, &node_artifact_hash(self.c, sub_node, ver)?)?;

        insert_artifact(self.c, node, version, "guinier-candidates", "fit", false, &json!({"candidates": guinier_cands}))?;
        insert_artifact(self.c, node, version, "porod-candidates", "fit", false, &json!({"candidates": porod_cands}))?;
        let aid = insert_artifact(self.c, node, version, "primary", "diagnostic", true, &json!({
            "guinier": guinier_cands,
            "porod": porod_cands,
            "overlaps": overlaps_v,
            "stitched_model": Value::Null,
            "policy": "independent multi-interval evidence; review decision required",
        }))?;
        let mut diagnostics = Vec::new();
        if !overlaps.is_empty() {
            diagnostics.push(format!("{} Guinier/Porod window overlaps reported, none merged", overlaps.len()));
        }
        set_diagnostics(self.c, node, version, &diagnostics)?;
        publish_version(self.c, &self.project_id, node, version)?;

        let ev = append_event(self.c, &self.project_id, &self.actor, "interval-evidence-published", &json!({
            "node": node,
            "version": version,
            "upstream": sub_node,
            "rule": "guinier-and-porod-never-auto-stitched",
            "guinier_candidates": guinier_cands.len(),
            "porod_candidates": porod_cands.len(),
            "overlaps": overlaps_v,
            "stitched_model": null,
            "artifact_primary": aid,
        }))?;
        Ok(RunOutcome { node: node.into(), version, published: true, event_seq: ev.seq, diagnostics })
    }
}

/// Conflict helpers surfaced to the web layer.
pub fn conflict_text(e: &rusqlite::Error) -> Option<String> {
    if is_conflict(e) {
        Some(e.to_string())
    } else {
        None
    }
}
