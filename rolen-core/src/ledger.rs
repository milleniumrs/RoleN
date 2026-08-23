//! SQLite ledger (PRD FR-4.6 / FR-14.2): token usage, sessions, ticket journal.

use crate::config;
use crate::error::CoreError;
use crate::types::LedgerEntry;
use rusqlite::Connection;
use std::path::Path;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS ledger_entries (
    id          TEXT PRIMARY KEY,
    session_id  TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    tokens_in     INTEGER NOT NULL DEFAULT 0,
    -- subsets of tokens_in: cache reads, and cache writes by TTL
    tokens_cached INTEGER NOT NULL DEFAULT 0,
    tokens_cache_write_5m INTEGER NOT NULL DEFAULT 0,
    tokens_cache_write_1h INTEGER NOT NULL DEFAULT 0,
    tokens_out  INTEGER NOT NULL DEFAULT 0,
    cost        REAL    NOT NULL DEFAULT 0,
    latency_ms  INTEGER,
    ts          TEXT    NOT NULL
);
CREATE TABLE IF NOT EXISTS sessions (
    id              TEXT PRIMARY KEY,
    task_id         TEXT,
    provider_id     TEXT NOT NULL,
    model           TEXT NOT NULL,
    state           TEXT NOT NULL,
    tokens_in       INTEGER NOT NULL DEFAULT 0,
    tokens_out      INTEGER NOT NULL DEFAULT 0,
    cost            REAL    NOT NULL DEFAULT 0,
    started         TEXT    NOT NULL,
    transcript_path TEXT
);
CREATE TABLE IF NOT EXISTS write_tickets (
    id        TEXT PRIMARY KEY,
    task_id   TEXT NOT NULL,
    path      TEXT NOT NULL,
    op        TEXT NOT NULL,
    payload   TEXT NOT NULL,
    base_hash TEXT,
    state     TEXT NOT NULL,
    ts        TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_ledger_ts    ON ledger_entries(ts);
CREATE INDEX IF NOT EXISTS idx_ledger_prov  ON ledger_entries(provider_id);
CREATE INDEX IF NOT EXISTS idx_sessions_ts  ON sessions(started);
CREATE INDEX IF NOT EXISTS idx_tickets_ts   ON write_tickets(ts);
";

pub struct Ledger {
    conn: Connection,
}

impl Ledger {
    pub fn open(path: &Path) -> Result<Self, CoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        let ledger = Self { conn };
        ledger.init()?;
        Ok(ledger)
    }

    pub fn open_default() -> Result<Self, CoreError> {
        Self::open(&config::ledger_file()?)
    }

    fn init(&self) -> Result<(), CoreError> {
        self.conn.execute_batch(SCHEMA)?;
        self.migrate()?;
        Ok(())
    }

    /// Bring an older database up to the current schema.
    ///
    /// `CREATE TABLE IF NOT EXISTS` does nothing to a table that already
    /// exists, so columns added after a release need an explicit ALTER. Each
    /// step is guarded by a column check and is safe to run repeatedly.
    fn migrate(&self) -> Result<(), CoreError> {
        for column in [
            "tokens_cached",
            "tokens_cache_write_5m",
            "tokens_cache_write_1h",
        ] {
            if !self.has_column("ledger_entries", column)? {
                self.conn.execute_batch(&format!(
                    "ALTER TABLE ledger_entries ADD COLUMN {column} INTEGER NOT NULL DEFAULT 0"
                ))?;
            }
        }
        Ok(())
    }

    fn has_column(&self, table: &str, column: &str) -> Result<bool, CoreError> {
        let mut stmt = self.conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let name: String = row.get(1)?;
            if name == column {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn record(&self, e: &LedgerEntry) -> Result<(), CoreError> {
        self.conn.execute(
            "INSERT INTO ledger_entries
             (id, session_id, provider_id, tokens_in, tokens_cached,
              tokens_cache_write_5m, tokens_cache_write_1h, tokens_out,
              cost, latency_ms, ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                e.id,
                e.session_id,
                e.provider_id,
                e.usage.input,
                e.usage.cache_read,
                e.usage.cache_write_5m,
                e.usage.cache_write_1h,
                e.usage.output,
                e.cost,
                e.latency_ms.map(|v| v as i64),
                e.ts.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn count_entries(&self) -> Result<u64, CoreError> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM ledger_entries", [], |row| row.get(0))?;
        Ok(n as u64)
    }

    /// Insert or update a session row (PRD FR-9).
    pub fn upsert_session(&self, s: &crate::types::Session) -> Result<(), CoreError> {
        self.conn.execute(
            "INSERT INTO sessions
             (id, task_id, provider_id, model, state, tokens_in, tokens_out, cost, started, transcript_path)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET
               task_id = excluded.task_id,
               provider_id = excluded.provider_id,
               model = excluded.model,
               state = excluded.state,
               tokens_in = excluded.tokens_in,
               tokens_out = excluded.tokens_out,
               cost = excluded.cost,
               transcript_path = excluded.transcript_path",
            rusqlite::params![
                s.id,
                s.task_id,
                s.provider_id,
                s.model,
                format!("{:?}", s.state).to_lowercase(),
                s.tokens_in,
                s.tokens_out,
                s.cost,
                s.started.to_rfc3339(),
                s.transcript_path.as_ref().map(|p| p.display().to_string()),
            ],
        )?;
        Ok(())
    }

    pub fn count_sessions_by_state(&self, state: &str) -> Result<u64, CoreError> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM sessions WHERE state = ?1",
            [state],
            |row| row.get(0),
        )?;
        Ok(n as u64)
    }

    /// Most recent sessions, newest first (dashboard, FR-9.1).
    pub fn recent_sessions(&self, limit: usize) -> Result<Vec<crate::types::Session>, CoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, task_id, provider_id, model, state, tokens_in, tokens_out, cost, started, transcript_path
             FROM sessions ORDER BY started DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |row| {
            let state_s: String = row.get(4)?;
            let started_s: String = row.get(8)?;
            Ok(crate::types::Session {
                id: row.get(0)?,
                task_id: row.get(1)?,
                provider_id: row.get(2)?,
                model: row.get(3)?,
                state: match state_s.as_str() {
                    "starting" => crate::types::SessionState::Starting,
                    "running" => crate::types::SessionState::Running,
                    "paused" => crate::types::SessionState::Paused,
                    "migrating" => crate::types::SessionState::Migrating,
                    "done" => crate::types::SessionState::Done,
                    _ => crate::types::SessionState::Failed,
                },
                tokens_in: row.get::<_, i64>(5)? as u64,
                tokens_out: row.get::<_, i64>(6)? as u64,
                cost: row.get(7)?,
                started: chrono::DateTime::parse_from_rfc3339(&started_s)
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now()),
                transcript_path: row.get::<_, Option<String>>(9)?.map(Into::into),
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    // ------------------------------------------------------- write tickets

    pub fn insert_ticket(&self, t: &crate::types::WriteTicket) -> Result<(), CoreError> {
        self.conn.execute(
            "INSERT INTO write_tickets (id, task_id, path, op, payload, base_hash, state, ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO NOTHING",
            rusqlite::params![
                t.id,
                t.task_id,
                t.path.display().to_string(),
                format!("{:?}", t.op).to_lowercase(),
                t.payload,
                t.base_hash,
                format!("{:?}", t.state).to_lowercase(),
                t.ts.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn set_ticket_state(
        &self,
        id: &str,
        state: crate::types::TicketState,
    ) -> Result<(), CoreError> {
        self.conn.execute(
            "UPDATE write_tickets SET state = ?2 WHERE id = ?1",
            rusqlite::params![id, format!("{:?}", state).to_lowercase()],
        )?;
        Ok(())
    }

    /// (applied, rejected, queued) counts since a timestamp (RFC3339).
    pub fn ticket_counts_since(&self, since_rfc3339: &str) -> Result<(u64, u64, u64), CoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT state, COUNT(*) FROM write_tickets WHERE ts >= ?1 GROUP BY state")?;
        let mut applied = 0;
        let mut rejected = 0;
        let mut queued = 0;
        let rows = stmt.query_map([since_rfc3339], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (state, n) = row?;
            match state.as_str() {
                "applied" => applied = n as u64,
                "rejected" => rejected = n as u64,
                _ => queued = n as u64,
            }
        }
        Ok((applied, rejected, queued))
    }

    /// Insert + delete a throwaway row. Used by `config doctor`.
    pub fn probe(&self) -> Result<(), CoreError> {
        self.conn.execute(
            "INSERT INTO ledger_entries
             (id, session_id, provider_id, tokens_in, tokens_out, cost, latency_ms, ts)
             VALUES ('__probe__', '__probe__', '__probe__', 0, 0, 0, NULL, '1970-01-01T00:00:00Z')
             ON CONFLICT(id) DO NOTHING",
            [],
        )?;
        self.conn
            .execute("DELETE FROM ledger_entries WHERE id = '__probe__'", [])?;
        Ok(())
    }

    /// Aggregate usage since a timestamp (RFC3339), optionally per provider.
    pub fn usage_since(
        &self,
        provider_id: Option<&str>,
        since_rfc3339: &str,
    ) -> Result<UsageSummary, CoreError> {
        let (sql, params): (&str, Vec<Box<dyn rusqlite::ToSql>>) = match provider_id {
            Some(p) => (
                "SELECT COALESCE(SUM(tokens_in),0), COALESCE(SUM(tokens_out),0),
                        COALESCE(SUM(cost),0), COUNT(*)
                 FROM ledger_entries WHERE ts >= ?1 AND provider_id = ?2",
                vec![Box::new(since_rfc3339.to_string()), Box::new(p.to_string())],
            ),
            None => (
                "SELECT COALESCE(SUM(tokens_in),0), COALESCE(SUM(tokens_out),0),
                        COALESCE(SUM(cost),0), COUNT(*)
                 FROM ledger_entries WHERE ts >= ?1",
                vec![Box::new(since_rfc3339.to_string())],
            ),
        };
        let mut stmt = self.conn.prepare(sql)?;
        let summary = stmt.query_row(rusqlite::params_from_iter(params.iter()), |row| {
            Ok(UsageSummary {
                tokens_in: row.get::<_, i64>(0)? as u64,
                tokens_out: row.get::<_, i64>(1)? as u64,
                cost: row.get::<_, f64>(2)?,
                requests: row.get::<_, i64>(3)? as u64,
            })
        })?;
        Ok(summary)
    }

    /// Usage since local midnight (UTC approximation of "today").
    pub fn usage_today(&self, provider_id: Option<&str>) -> Result<UsageSummary, CoreError> {
        let today = chrono::Utc::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let since = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(today, chrono::Utc);
        self.usage_since(provider_id, &since.to_rfc3339())
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct UsageSummary {
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost: f64,
    pub requests: u64,
}

impl UsageSummary {
    pub fn total_tokens(&self) -> u64 {
        self.tokens_in + self.tokens_out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    /// A scratch database path that cleans itself up.
    struct TempDb(std::path::PathBuf);

    impl TempDb {
        fn new(tag: &str) -> Self {
            let mut p = std::env::temp_dir();
            p.push(format!(
                "rolen-ledger-{tag}-{}.sqlite3",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            Self(p)
        }
    }

    impl Drop for TempDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn entry(id: &str, usage: crate::pricing::Tokens) -> LedgerEntry {
        LedgerEntry {
            id: id.into(),
            session_id: "s1".into(),
            provider_id: "kimi".into(),
            usage,
            cost: 1.5,
            latency_ms: Some(42),
            ts: chrono::Utc::now(),
        }
    }

    /// The schema as it shipped in v0.2.0, before cached pricing.
    const V020_SCHEMA: &str = "
    CREATE TABLE ledger_entries (
        id TEXT PRIMARY KEY, session_id TEXT NOT NULL, provider_id TEXT NOT NULL,
        tokens_in INTEGER NOT NULL DEFAULT 0, tokens_out INTEGER NOT NULL DEFAULT 0,
        cost REAL NOT NULL DEFAULT 0, latency_ms INTEGER, ts TEXT NOT NULL);
    ";

    const CACHE_COLUMNS: [&str; 3] = [
        "tokens_cached",
        "tokens_cache_write_5m",
        "tokens_cache_write_1h",
    ];

    #[test]
    fn a_fresh_database_has_every_cache_column() {
        let db = TempDb::new("fresh");
        let l = Ledger::open(&db.0).unwrap();
        for c in CACHE_COLUMNS {
            assert!(l.has_column("ledger_entries", c).unwrap(), "missing {c}");
        }
    }

    #[test]
    fn an_existing_database_is_migrated_in_place() {
        let db = TempDb::new("migrate");
        // Lay down the old schema with a row in it, as an upgrader would have.
        {
            let conn = Connection::open(&db.0).unwrap();
            conn.execute_batch(V020_SCHEMA).unwrap();
            conn.execute(
                "INSERT INTO ledger_entries (id, session_id, provider_id, tokens_in, tokens_out, cost, ts)
                 VALUES ('old', 's0', 'kimi', 100, 20, 0.0, '2026-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
        }

        let l = Ledger::open(&db.0).unwrap();
        for c in CACHE_COLUMNS {
            assert!(l.has_column("ledger_entries", c).unwrap(), "missing {c}");
        }

        // The pre-existing row survives and reads back as "no cache traffic",
        // which bills it exactly as it was billed before.
        let cached: i64 = l
            .conn
            .query_row(
                "SELECT tokens_cached + tokens_cache_write_5m + tokens_cache_write_1h
                 FROM ledger_entries WHERE id = 'old'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cached, 0);
        assert_eq!(l.count_entries().unwrap(), 1);
    }

    #[test]
    fn migration_is_safe_to_run_twice() {
        let db = TempDb::new("twice");
        Ledger::open(&db.0).unwrap();
        // Re-opening runs init() again; a second ALTER would error.
        let l = Ledger::open(&db.0).unwrap();
        for c in CACHE_COLUMNS {
            assert!(l.has_column("ledger_entries", c).unwrap(), "missing {c}");
        }
    }

    #[test]
    fn a_partially_migrated_database_gains_only_what_it_lacks() {
        // v0.3.0 added tokens_cached; a database from that build must gain
        // the two write columns without tripping over the one it has.
        let db = TempDb::new("partial");
        {
            let conn = Connection::open(&db.0).unwrap();
            conn.execute_batch(V020_SCHEMA).unwrap();
            conn.execute_batch(
                "ALTER TABLE ledger_entries ADD COLUMN tokens_cached INTEGER NOT NULL DEFAULT 0",
            )
            .unwrap();
        }
        let l = Ledger::open(&db.0).unwrap();
        for c in CACHE_COLUMNS {
            assert!(l.has_column("ledger_entries", c).unwrap(), "missing {c}");
        }
    }

    #[test]
    fn every_cache_bucket_round_trips_through_the_ledger() {
        let db = TempDb::new("roundtrip");
        let l = Ledger::open(&db.0).unwrap();
        l.record(&entry(
            "e1",
            crate::pricing::Tokens {
                input: 1000,
                cache_read: 500,
                cache_write_5m: 200,
                cache_write_1h: 100,
                output: 10,
            },
        ))
        .unwrap();
        let (read, w5m, w1h): (i64, i64, i64) = l
            .conn
            .query_row(
                "SELECT tokens_cached, tokens_cache_write_5m, tokens_cache_write_1h
                 FROM ledger_entries WHERE id = 'e1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((read, w5m, w1h), (500, 200, 100));
    }

    #[test]
    fn cache_buckets_do_not_inflate_the_usage_total() {
        // tokens_in already counts every cache bucket, so quota totals must
        // not double-count them.
        let db = TempDb::new("totals");
        let l = Ledger::open(&db.0).unwrap();
        l.record(&entry(
            "e1",
            crate::pricing::Tokens {
                input: 1000,
                cache_read: 500,
                cache_write_5m: 200,
                cache_write_1h: 100,
                output: 10,
            },
        ))
        .unwrap();
        let u = l.usage_since(Some("kimi"), "2000-01-01T00:00:00Z").unwrap();
        assert_eq!(u.tokens_in, 1000);
        assert_eq!(u.total_tokens(), 1010);
    }
}
