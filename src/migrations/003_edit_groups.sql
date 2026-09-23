-- 编辑分支头：两位使用者从同一版本编辑时产生可解决冲突
CREATE TABLE IF NOT EXISTS edit_heads (
    edit_group TEXT PRIMARY KEY,
    head_seq   INTEGER NOT NULL
);

INSERT OR IGNORE INTO edit_heads(edit_group, head_seq) VALUES ('main', 0);

-- 归档身份校验表
CREATE TABLE IF NOT EXISTS archive_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
