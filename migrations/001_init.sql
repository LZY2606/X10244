CREATE TABLE IF NOT EXISTS events (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    actor TEXT NOT NULL,
    revision INTEGER NOT NULL UNIQUE,
    payload TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS event_parents (
    event_id TEXT NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    parent_id TEXT NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    PRIMARY KEY (event_id, parent_id)
);

CREATE INDEX IF NOT EXISTS idx_event_parents_parent ON event_parents(parent_id);
CREATE INDEX IF NOT EXISTS idx_events_revision ON events(revision);
