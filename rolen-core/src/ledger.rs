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
    tokens_in   INTEGER NOT NULL DEFAULT 0,
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
        Ok(())
    }

    pub fn record(&self, e: &LedgerEntry) -> Result<(), CoreError> {
        self.conn.execute(
            "INSERT INTO ledger_entries
             (id, session_id, provider_id, tokens_in, tokens_out, cost, latency_ms, ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                e.id,
                e.session_id,
                e.provider_id,
                e.tokens_in,
                e.tokens_out,
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
