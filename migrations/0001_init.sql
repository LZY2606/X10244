-- SAXS bench schema v1: provenance-preserving, versioned derivation store.
PRAGMA foreign_keys = ON;

CREATE TABLE meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE projects (
  project_id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  created_at TEXT NOT NULL
);

CREATE TABLE instruments (
  instrument_id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  name TEXT NOT NULL,
  spec_json TEXT NOT NULL,
  created_at TEXT NOT NULL
);

CREATE TABLE masks (
  mask_id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  name TEXT NOT NULL,
  origin TEXT NOT NULL,
  locator TEXT NOT NULL,
  checksum TEXT NOT NULL,
  created_at TEXT NOT NULL
);

-- Raw material. A revision inserts a new (record_id, version) row; old rows are
-- never overwritten. Numeric canonical values sit next to the raw text so the
-- original representation is always recoverable.
CREATE TABLE raw_records (
  record_id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  role TEXT NOT NULL,                 -- 'sample' | 'buffer'
  name TEXT NOT NULL,
  version INTEGER NOT NULL,
  superseded_by TEXT,
  thickness_json TEXT,
  exposure_json TEXT NOT NULL,
  transmission_json TEXT NOT NULL,
  instrument_id TEXT,
  mask_id TEXT NOT NULL,
  source_file TEXT NOT NULL,
  created_at TEXT NOT NULL,
  UNIQUE(project_id, role, name, version)
);

CREATE TABLE raw_points (
  record_id TEXT NOT NULL,
  version INTEGER NOT NULL,
  ordinal INTEGER NOT NULL,
  q_raw TEXT NOT NULL,
  i_raw TEXT NOT NULL,
  sig_raw TEXT NOT NULL,
  q REAL NOT NULL,
  i REAL NOT NULL,
  sig REAL NOT NULL,
  flags_json TEXT NOT NULL DEFAULT '[]',
  PRIMARY KEY (record_id, version, ordinal)
);

CREATE TABLE buffer_candidates (
  project_id TEXT NOT NULL,
  sample_record_id TEXT NOT NULL,
  buffer_record_id TEXT NOT NULL,
  ordinal INTEGER NOT NULL,
  PRIMARY KEY (project_id, sample_record_id, buffer_record_id)
);

-- Derivation slots. Each recomputation appends a new immutable version; the
-- current pointer only moves once a version is complete.
CREATE TABLE derivations (
  node TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  kind TEXT NOT NULL,                 -- normalization | subtraction | guinier | porod | candtest
  title TEXT NOT NULL,
  current_version INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE node_config (
  node TEXT PRIMARY KEY,
  config_json TEXT NOT NULL,
  rev INTEGER NOT NULL DEFAULT 1,
  updated_by TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE derivation_versions (
  node TEXT NOT NULL,
  version INTEGER NOT NULL,
  status TEXT NOT NULL,               -- pending | complete | failed | aborted
  inputs_hash TEXT NOT NULL,
  config_json TEXT NOT NULL,
  diagnostics_json TEXT NOT NULL DEFAULT '[]',
  created_at TEXT NOT NULL,
  completed_at TEXT,
  checksum TEXT,
  PRIMARY KEY (node, version)
);

CREATE TABLE version_deps (
  node TEXT NOT NULL,
  version INTEGER NOT NULL,
  dep_kind TEXT NOT NULL,             -- raw | node | instrument | mask | config
  ref_id TEXT NOT NULL,
  ref_version INTEGER NOT NULL,
  ref_hash TEXT NOT NULL,
  PRIMARY KEY (node, version, dep_kind, ref_id)
);

CREATE TABLE artifacts (
  artifact_id TEXT PRIMARY KEY,
  node TEXT NOT NULL,
  version INTEGER NOT NULL,
  candidate_key TEXT NOT NULL DEFAULT 'primary',
  kind TEXT NOT NULL,                 -- curve | fit | residual_test | diagnostic
  adopted INTEGER NOT NULL DEFAULT 0,
  payload_json TEXT NOT NULL,
  checksum TEXT NOT NULL,
  created_at TEXT NOT NULL
);

CREATE TABLE node_decisions (
  node TEXT PRIMARY KEY,
  adopted_artifact_id TEXT,
  decision_rev INTEGER NOT NULL DEFAULT 0,
  action TEXT,
  decided_by TEXT,
  rationale TEXT,
  updated_at TEXT
);

CREATE TABLE events (
  event_id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  seq INTEGER NOT NULL,
  ts TEXT NOT NULL,
  actor TEXT NOT NULL,
  event_type TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  PRIMARY KEY (project_id, seq)
);

CREATE TABLE pointers_log (
  project_id TEXT NOT NULL,
  seq INTEGER NOT NULL,
  node TEXT NOT NULL,
  from_version INTEGER NOT NULL,
  to_version INTEGER NOT NULL,
  PRIMARY KEY (project_id, seq, node)
);

-- Per-point lineage makes every displayed value traceable to source curves.
CREATE TABLE point_lineage (
  point_id TEXT PRIMARY KEY,
  artifact_id TEXT NOT NULL,
  parent_points_json TEXT NOT NULL
);
