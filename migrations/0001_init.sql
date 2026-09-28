PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS events (
  event_id TEXT PRIMARY KEY,
  seq INTEGER NOT NULL UNIQUE,
  event_type TEXT NOT NULL,
  parent_event_id TEXT,
  branch_id TEXT NOT NULL,
  actor TEXT NOT NULL,
  summary TEXT NOT NULL,
  payload TEXT NOT NULL,
  created_at TEXT NOT NULL
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS instrument_responses (
  id TEXT PRIMARY KEY,
  version_no INTEGER NOT NULL,
  logical_id TEXT NOT NULL,
  label TEXT NOT NULL,
  source TEXT NOT NULL,
  gain_raw TEXT NOT NULL,
  gain REAL,
  unit TEXT NOT NULL,
  sigma_relative REAL,
  coord_basis TEXT NOT NULL,
  content_hash TEXT NOT NULL,
  event_id TEXT NOT NULL
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS mask_sources (
  id TEXT PRIMARY KEY,
  version_no INTEGER NOT NULL,
  logical_id TEXT NOT NULL,
  label TEXT NOT NULL,
  source TEXT NOT NULL,
  beamstop_q_max_raw TEXT NOT NULL,
  beamstop_q_max REAL,
  q_unit TEXT NOT NULL,
  coord_basis TEXT NOT NULL,
  content_hash TEXT NOT NULL,
  event_id TEXT NOT NULL
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS raw_curves (
  id TEXT PRIMARY KEY,
  logical_id TEXT NOT NULL,
  version_no INTEGER NOT NULL,
  role TEXT NOT NULL,
  label TEXT NOT NULL,
  q_unit TEXT NOT NULL,
  i_unit TEXT NOT NULL,
  coord_basis TEXT NOT NULL,
  exposure_raw TEXT NOT NULL,
  exposure_s REAL,
  transmission_raw TEXT NOT NULL,
  transmission REAL,
  thickness_raw TEXT NOT NULL,
  thickness_cm REAL,
  response_id TEXT,
  mask_id TEXT,
  source_json TEXT NOT NULL,
  content_hash TEXT NOT NULL,
  event_id TEXT NOT NULL,
  UNIQUE(logical_id, version_no)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS raw_points (
  id TEXT PRIMARY KEY,
  curve_id TEXT NOT NULL,
  seq INTEGER NOT NULL,
  q_raw TEXT NOT NULL,
  i_raw TEXT NOT NULL,
  sigma_raw TEXT NOT NULL,
  q_value REAL,
  i_value REAL,
  sigma_value REAL,
  diagnostic TEXT NOT NULL,
  UNIQUE(curve_id, seq)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS buffer_candidates (
  id TEXT PRIMARY KEY,
  sample_curve_id TEXT NOT NULL,
  buffer_curve_id TEXT NOT NULL,
  label TEXT NOT NULL,
  alpha_raw TEXT NOT NULL,
  alpha_value REAL,
  sigma_alpha_relative REAL NOT NULL,
  priority INTEGER NOT NULL,
  source TEXT NOT NULL,
  event_id TEXT NOT NULL
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS rule_params (
  id TEXT PRIMARY KEY,
  version_no INTEGER NOT NULL,
  label TEXT NOT NULL,
  interpolation TEXT NOT NULL,
  overlap_tolerance REAL NOT NULL,
  fit_q_min REAL NOT NULL,
  bg_scale_sigma_relative REAL NOT NULL,
  chi2_limit REAL NOT NULL,
  negative_policy TEXT NOT NULL,
  interval_policy TEXT NOT NULL,
  content_hash TEXT NOT NULL,
  event_id TEXT NOT NULL
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS derivations (
  id TEXT PRIMARY KEY,
  lineage_id TEXT NOT NULL,
  version_no INTEGER NOT NULL,
  kind TEXT NOT NULL,
  status TEXT NOT NULL,
  rule_id TEXT,
  inputs_json TEXT NOT NULL,
  output_json TEXT NOT NULL,
  diagnostics_json TEXT NOT NULL,
  content_hash TEXT NOT NULL,
  event_id TEXT NOT NULL,
  created_seq INTEGER NOT NULL,
  UNIQUE(lineage_id, version_no)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS derivation_points (
  id TEXT PRIMARY KEY,
  derivation_id TEXT NOT NULL,
  seq INTEGER NOT NULL,
  q_value REAL,
  value REAL,
  sigma_norm REAL,
  sigma_bg_scale REAL,
  sigma_total REAL,
  valid INTEGER NOT NULL,
  point_payload_json TEXT NOT NULL,
  UNIQUE(derivation_id, seq)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS derivation_refs (
  derivation_id TEXT NOT NULL,
  role TEXT NOT NULL,
  ref_kind TEXT NOT NULL,
  ref_id TEXT NOT NULL,
  point_id TEXT,
  content_hash TEXT NOT NULL,
  PRIMARY KEY(derivation_id, role, ref_kind, ref_id, point_id)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS decisions (
  id TEXT PRIMARY KEY,
  event_id TEXT NOT NULL,
  subject_kind TEXT NOT NULL,
  subject_id TEXT NOT NULL,
  action TEXT NOT NULL,
  chosen_id TEXT,
  actor TEXT NOT NULL,
  reason TEXT NOT NULL,
  status TEXT NOT NULL,
  conflict_of_event_id TEXT,
  resolution_of_event_id TEXT,
  parent_event_id TEXT,
  created_seq INTEGER NOT NULL
) WITHOUT ROWID;

CREATE INDEX idx_raw_points_curve ON raw_points(curve_id);
CREATE INDEX idx_derivations_lineage ON derivations(lineage_id, version_no);
CREATE INDEX idx_derivation_points_derivation ON derivation_points(derivation_id, seq);
CREATE INDEX idx_derivation_refs_ref ON derivation_refs(ref_kind, ref_id);
CREATE INDEX idx_decisions_subject ON decisions(subject_kind, subject_id, created_seq);
