-- 初始模式：原始材料（全历史修订）+ 派生版本图 + 事件日志
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS materials (
    id            TEXT NOT NULL,
    revision      INTEGER NOT NULL,
    kind          TEXT NOT NULL,
    label         TEXT NOT NULL,
    source_ref    TEXT NOT NULL,
    json          TEXT NOT NULL,
    superseded_by INTEGER,
    created_event INTEGER NOT NULL,
    PRIMARY KEY (id, revision)
);

CREATE TABLE IF NOT EXISTS material_heads (
    id       TEXT PRIMARY KEY,
    revision INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS node_versions (
    node_id      TEXT NOT NULL,
    ordinal      INTEGER NOT NULL,
    node_kind    TEXT NOT NULL,
    version_id   TEXT NOT NULL UNIQUE,
    inputs_json  TEXT NOT NULL,
    rule         TEXT NOT NULL,
    params_json  TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    diag_json    TEXT NOT NULL,
    created_event INTEGER NOT NULL,
    PRIMARY KEY (node_id, ordinal)
);

CREATE TABLE IF NOT EXISTS node_pointers (
    node_id            TEXT PRIMARY KEY,
    current_version_id TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS decisions (
    seq         INTEGER PRIMARY KEY,
    node_id     TEXT NOT NULL,
    version_id  TEXT NOT NULL,
    decision    TEXT NOT NULL,
    reason      TEXT NOT NULL,
    rejected    INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS events (
    seq           INTEGER PRIMARY KEY AUTOINCREMENT,
    kind          TEXT NOT NULL,
    action        TEXT NOT NULL,
    summary       TEXT NOT NULL,
    author        TEXT NOT NULL DEFAULT 'reviewer',
    edit_group    TEXT NOT NULL DEFAULT 'main',
    expected_head INTEGER,
    supersedes    TEXT NOT NULL DEFAULT '[]',
    outcome       TEXT NOT NULL,
    affected_json TEXT NOT NULL DEFAULT '[]',
    versions_json TEXT NOT NULL DEFAULT '[]',
    rules_json    TEXT NOT NULL DEFAULT '[]',
    codes_json    TEXT NOT NULL DEFAULT '[]',
    payload_json  TEXT NOT NULL DEFAULT '{}',
    created_at    TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_node_versions_ver ON node_versions(version_id);
CREATE INDEX IF NOT EXISTS idx_decisions_node ON decisions(node_id, version_id);
