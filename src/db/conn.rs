use rusqlite::Connection;
use std::sync::{Mutex, MutexGuard};

pub struct Store {
    conn: Mutex<Connection>,
}

pub type Guard<'a> = MutexGuard<'a, Connection>;

impl Store {
    pub fn open(path: &str) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "busy_timeout", 10_000)?;
        let store = Store { conn: Mutex::new(conn) };
        store.migrate()?;
        Ok(store)
    }

    pub fn in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let store = Store { conn: Mutex::new(conn) };
        store.migrate()?;
        Ok(store)
    }

    pub fn lock(&self) -> Guard<'_> {
        self.conn.lock().expect("store mutex poisoned")
    }

    fn migrate(&self) -> rusqlite::Result<()> {
        let mut c = self.conn.lock().unwrap();
        let current: i64 =
            c.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap_or(0);
        let files = [
            (1, include_str!("../../migrations/0001_init.sql")),
            (2, include_str!("../../migrations/0002_event_review.sql")),
        ];
        for (v, sql) in files {
            if current < v {
                let tx = c.transaction()?;
                tx.execute_batch(sql)?;
                tx.execute_batch(&format!("PRAGMA user_version = {v}"))?;
                tx.commit()?;
            }
        }
        Ok(())
    }

    pub fn schema_version(&self) -> i64 {
        self.lock().query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap_or(0)
    }
}
