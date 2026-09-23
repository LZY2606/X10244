-- 暂存写入：支持“崩溃后只见旧版本或完整新版本”
CREATE TABLE IF NOT EXISTS staged_write (
    tx_id      TEXT PRIMARY KEY,
    created_at TEXT NOT NULL,
    note       TEXT NOT NULL DEFAULT ''
);

-- 已采纳决定（accept/retune）历史，用于导出和审计（001 的 decisions 为未拒绝候选登记）。
CREATE TABLE IF NOT EXISTS adopted_decisions (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    event_seq    INTEGER NOT NULL,
    node_id      TEXT NOT NULL,
    version_id   TEXT NOT NULL,
    decision     TEXT NOT NULL,
    reason       TEXT NOT NULL,
    edit_group   TEXT NOT NULL DEFAULT 'main'
);
