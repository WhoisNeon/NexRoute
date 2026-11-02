use std::path::Path;
use std::sync::Mutex;

use chrono::{SecondsFormat, Utc};
use rusqlite::{named_params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::config::model::Outbound;
use crate::config::settings::Settings;
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfigStatus {
    Untested,
    Alive,
    Dead,
}

impl ConfigStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Untested => "untested",
            Self::Alive => "alive",
            Self::Dead => "dead",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "alive" => Self::Alive,
            "dead" => Self::Dead,
            _ => Self::Untested,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConfigRecord {
    pub id: i64,
    pub raw_uri: String,
    pub outbound: Outbound,
    pub added_at: String,
    pub last_tested_at: Option<String>,
    pub last_real_delay_ms: Option<i64>,
    pub status: ConfigStatus,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigView {
    pub id: i64,
    pub protocol: String,
    pub protocol_label: String,
    pub name: String,
    pub server: String,
    pub port: u16,
    pub transport: String,
    pub security: String,
    pub raw_uri: String,
    pub added_at: String,
    pub last_tested_at: Option<String>,
    pub last_real_delay_ms: Option<i64>,
    pub status: ConfigStatus,
    pub last_error: Option<String>,
}

impl ConfigRecord {
    pub fn view(&self) -> ConfigView {
        let security = match &self.outbound.tls {
            Some(tls) if tls.reality.is_some() => "reality",
            Some(_) => "tls",
            None => "none",
        };
        ConfigView {
            id: self.id,
            protocol: self.outbound.protocol.as_str().to_string(),
            protocol_label: self.outbound.protocol.label().to_string(),
            name: self.outbound.display_name(),
            server: self.outbound.server.clone(),
            port: self.outbound.port,
            transport: self.outbound.transport_kind().to_string(),
            security: security.to_string(),
            raw_uri: self.raw_uri.clone(),
            added_at: self.added_at.clone(),
            last_tested_at: self.last_tested_at.clone(),
            last_real_delay_ms: self.last_real_delay_ms,
            status: self.status,
            last_error: self.last_error.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub total: usize,
    pub alive: usize,
    pub dead: usize,
    pub untested: usize,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ListQuery {
    pub search: String,
    pub status: String,
    pub protocol: String,
    pub sort: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvalidUri {
    pub input: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOutcome {
    pub added: usize,
    pub duplicates: usize,
    pub invalid: Vec<InvalidUri>,
}

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> AppResult<Self> {
        let conn = Connection::open(path)?;
        Self::from_conn(conn)
    }

    #[cfg(test)]
    pub fn in_memory() -> AppResult<Self> {
        Self::from_conn(Connection::open_in_memory()?)
    }

    fn from_conn(conn: Connection) -> AppResult<Self> {
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS configs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                protocol TEXT NOT NULL,
                raw_uri TEXT NOT NULL UNIQUE,
                parsed_outbound_json TEXT NOT NULL,
                added_at TEXT NOT NULL,
                last_tested_at TEXT,
                last_real_delay_ms INTEGER,
                last_error TEXT,
                status TEXT NOT NULL DEFAULT 'untested'
             );
             CREATE INDEX IF NOT EXISTS idx_configs_status ON configs(status);
             CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
             );",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn import(&self, outbounds: &[(String, Outbound)]) -> AppResult<(usize, usize)> {
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let now = now_iso();
        let mut added = 0usize;
        {
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO configs (protocol, raw_uri, parsed_outbound_json, added_at)
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (raw, outbound) in outbounds {
                let json = serde_json::to_string(outbound)?;
                let changed = stmt.execute(rusqlite::params![
                    outbound.protocol.as_str(),
                    raw,
                    json,
                    now
                ])?;
                if changed > 0 {
                    added += 1;
                }
            }
        }
        tx.commit()?;
        Ok((added, outbounds.len().saturating_sub(added)))
    }

    pub fn list(&self, query: &ListQuery) -> AppResult<Vec<ConfigRecord>> {
        if !matches!(
            query.status.as_str(),
            "" | "alive" | "dead" | "untested"
        ) {
            return Err(AppError::msg("unknown status filter"));
        }
        if !matches!(
            query.protocol.as_str(),
            "" | "vless" | "vmess" | "ss" | "socks" | "http"
        ) {
            return Err(AppError::msg("unknown protocol filter"));
        }
        let order = match query.sort.as_str() {
            "" | "addedAt-desc" => "added_at DESC, id DESC",
            "addedAt-asc" => "added_at ASC, id ASC",
            "latency-asc" => "last_real_delay_ms IS NULL, last_real_delay_ms ASC",
            "latency-desc" => "last_real_delay_ms IS NULL, last_real_delay_ms DESC",
            "status-asc" => "status ASC, added_at DESC",
            _ => return Err(AppError::msg("unknown sort option")),
        };
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT id, protocol, raw_uri, parsed_outbound_json, added_at, last_tested_at,
                    last_real_delay_ms, last_error, status
             FROM configs
             WHERE (:search = '' OR raw_uri LIKE '%' || :search || '%'
                    OR parsed_outbound_json LIKE '%' || :search || '%')
               AND (:status = '' OR status = :status)
               AND (:protocol = '' OR protocol = :protocol)
             ORDER BY {order}"
        ))?;
        let rows = stmt
            .query_map(
                named_params! {
                    ":search": query.search.trim(),
                    ":status": query.status,
                    ":protocol": query.protocol,
                },
                raw_record,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().map(record_from_raw).collect()
    }

    pub fn all(&self) -> AppResult<Vec<ConfigRecord>> {
        self.list(&ListQuery::default())
    }

    pub fn counts(&self) -> AppResult<Counts> {
        let conn = self.lock();
        let mut counts = Counts {
            total: 0,
            alive: 0,
            dead: 0,
            untested: 0,
        };
        let mut stmt = conn.prepare("SELECT status, COUNT(*) FROM configs GROUP BY status")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (status, count) = row?;
            let count = count.max(0) as usize;
            counts.total += count;
            match ConfigStatus::parse(&status) {
                ConfigStatus::Alive => counts.alive += count,
                ConfigStatus::Dead => counts.dead += count,
                ConfigStatus::Untested => counts.untested += count,
            }
        }
        Ok(counts)
    }

    pub fn ids_matching(&self, query: &ListQuery) -> AppResult<Vec<i64>> {
        Ok(self.list(query)?.into_iter().map(|c| c.id).collect())
    }

    pub fn by_ids(&self, ids: &[i64]) -> AppResult<Vec<ConfigRecord>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = std::iter::repeat("?")
            .take(ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT id, protocol, raw_uri, parsed_outbound_json, added_at, last_tested_at,
                    last_real_delay_ms, last_error, status
             FROM configs WHERE id IN ({placeholders})"
        ))?;
        let params = rusqlite::params_from_iter(ids.iter());
        let rows = stmt
            .query_map(params, raw_record)?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().map(record_from_raw).collect()
    }

    pub fn alive(&self) -> AppResult<Vec<ConfigRecord>> {
        self.list(&ListQuery {
            status: "alive".into(),
            ..Default::default()
        })
    }

    pub fn set_result(
        &self,
        id: i64,
        alive: bool,
        delay_ms: Option<i64>,
        error: Option<&str>,
    ) -> AppResult<()> {
        let conn = self.lock();
        conn.execute(
            "UPDATE configs SET status = ?1, last_tested_at = ?2, last_real_delay_ms = ?3,
                    last_error = ?4 WHERE id = ?5",
            rusqlite::params![
                if alive { "alive" } else { "dead" },
                now_iso(),
                delay_ms,
                error,
                id
            ],
        )?;
        Ok(())
    }

    pub fn delete(&self, ids: &[i64]) -> AppResult<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let mut conn = self.lock();
        let tx = conn.transaction()?;
        let mut deleted = 0usize;
        {
            let mut stmt = tx.prepare("DELETE FROM configs WHERE id = ?1")?;
            for id in ids {
                deleted += stmt.execute(rusqlite::params![id])?;
            }
        }
        tx.commit()?;
        Ok(deleted)
    }

    pub fn delete_dead(&self) -> AppResult<usize> {
        let conn = self.lock();
        Ok(conn.execute("DELETE FROM configs WHERE status = 'dead'", [])?)
    }

    pub fn settings(&self) -> AppResult<Settings> {
        let conn = self.lock();
        let raw: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'settings'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        match raw {
            Some(value) => {
                let settings: Settings = serde_json::from_str(&value).unwrap_or_default();
                Ok(settings)
            }
            None => Ok(Settings::default()),
        }
    }

    pub fn save_settings(&self, settings: &Settings) -> AppResult<()> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('settings', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [serde_json::to_string(settings)?],
        )?;
        Ok(())
    }
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

struct RawRecord {
    id: i64,
    raw_uri: String,
    parsed: String,
    added_at: String,
    last_tested_at: Option<String>,
    last_real_delay_ms: Option<i64>,
    last_error: Option<String>,
    status: String,
}

fn raw_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawRecord> {
    Ok(RawRecord {
        id: row.get(0)?,
        raw_uri: row.get(2)?,
        parsed: row.get(3)?,
        added_at: row.get(4)?,
        last_tested_at: row.get(5)?,
        last_real_delay_ms: row.get(6)?,
        last_error: row.get(7)?,
        status: row.get(8)?,
    })
}

fn record_from_raw(raw: RawRecord) -> AppResult<ConfigRecord> {
    let outbound: Outbound = serde_json::from_str(&raw.parsed)
        .map_err(|e| AppError::msg(format!("stored configuration is corrupt: {e}")))?;
    Ok(ConfigRecord {
        id: raw.id,
        raw_uri: raw.raw_uri,
        outbound,
        added_at: raw.added_at,
        last_tested_at: raw.last_tested_at,
        last_real_delay_ms: raw.last_real_delay_ms,
        status: ConfigStatus::parse(&raw.status),
        last_error: raw.last_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::Protocol;

    fn sample(raw: &str) -> (String, Outbound) {
        (
            raw.to_string(),
            Outbound {
                protocol: Protocol::Socks,
                server: "127.0.0.1".into(),
                port: 1080,
                name: raw.to_string(),
                uuid: None,
                flow: None,
                alter_id: None,
                security: None,
                method: None,
                password: Some("pw".into()),
                username: Some("user".into()),
                plugin: None,
                plugin_opts: None,
                tls: None,
                transport: None,
            },
        )
    }

    #[test]
    fn imports_once_and_updates_status() {
        let store = Store::in_memory().unwrap();
        let (added, duplicates) = store
            .import(&[sample("socks://a"), sample("socks://b"), sample("socks://a")])
            .unwrap();
        assert_eq!((added, duplicates), (2, 1));
        let rows = store.all().unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.status == ConfigStatus::Untested));
        store.set_result(rows[0].id, false, None, Some("timeout")).unwrap();
        assert_eq!(store.counts().unwrap().dead, 1);
        assert_eq!(store.counts().unwrap().untested, 1);
        let dead = store
            .list(&ListQuery {
                status: "dead".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0].last_error.as_deref(), Some("timeout"));
    }

    #[test]
    fn delete_dead_never_touches_alive_or_untested() {
        let store = Store::in_memory().unwrap();
        store
            .import(&[sample("socks://a"), sample("socks://b"), sample("socks://c")])
            .unwrap();
        let rows = store.all().unwrap();
        store.set_result(rows[0].id, true, Some(120), None).unwrap();
        store.set_result(rows[1].id, false, None, Some("boom")).unwrap();
        assert_eq!(store.delete_dead().unwrap(), 1);
        let counts = store.counts().unwrap();
        assert_eq!((counts.alive, counts.dead, counts.untested), (1, 0, 1));
    }

    #[test]
    fn settings_round_trip() {
        let store = Store::in_memory().unwrap();
        assert_eq!(store.settings().unwrap(), Settings::default());
        let settings = Settings {
            test_url: "https://example.com/204".into(),
            timeout_ms: 4500,
            ..Default::default()
        };
        store.save_settings(&settings).unwrap();
        assert_eq!(store.settings().unwrap(), settings);
    }
}
