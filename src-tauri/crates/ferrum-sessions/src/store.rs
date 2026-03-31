use ferrum_core::types::Message;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub profile: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub is_pinned: bool,
    #[serde(default)]
    pub message_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_preview: Option<String>,
}

pub struct SessionStore {
    conn: Connection,
}

impl SessionStore {
    pub fn new() -> anyhow::Result<Self> {
        let db_path = Self::db_path();
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (
                id          TEXT PRIMARY KEY,
                name        TEXT NOT NULL,
                profile     TEXT NOT NULL,
                created_at  INTEGER NOT NULL,
                updated_at  INTEGER NOT NULL,
                is_pinned   INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS messages (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                role        TEXT NOT NULL,
                content     TEXT NOT NULL,
                tool_calls  TEXT,
                tool_name   TEXT,
                token_count INTEGER DEFAULT 0,
                created_at  INTEGER NOT NULL,
                is_compacted INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS cache_entries (
                id           TEXT PRIMARY KEY,
                prompt_hash  TEXT,
                response     TEXT NOT NULL,
                token_count  INTEGER DEFAULT 0,
                created_at   INTEGER NOT NULL,
                accessed_at  INTEGER NOT NULL,
                hit_count    INTEGER DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id);
            CREATE INDEX IF NOT EXISTS idx_cache_hash ON cache_entries(prompt_hash);",
        )?;

        Ok(Self { conn })
    }

    /// Create an in-memory session store (used for tests and as fallback when
    /// the on-disk database cannot be opened).
    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (
                id          TEXT PRIMARY KEY,
                name        TEXT NOT NULL,
                profile     TEXT NOT NULL,
                created_at  INTEGER NOT NULL,
                updated_at  INTEGER NOT NULL,
                is_pinned   INTEGER DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS messages (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                role        TEXT NOT NULL,
                content     TEXT NOT NULL,
                tool_calls  TEXT,
                tool_name   TEXT,
                token_count INTEGER DEFAULT 0,
                created_at  INTEGER NOT NULL,
                is_compacted INTEGER DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS cache_entries (
                id           TEXT PRIMARY KEY,
                prompt_hash  TEXT,
                response     TEXT NOT NULL,
                token_count  INTEGER DEFAULT 0,
                created_at   INTEGER NOT NULL,
                accessed_at  INTEGER NOT NULL,
                hit_count    INTEGER DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id);
            CREATE INDEX IF NOT EXISTS idx_cache_hash ON cache_entries(prompt_hash);",
        )?;
        Ok(Self { conn })
    }

    fn db_path() -> PathBuf {
        dirs_next::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("ferrum-chat")
            .join("ferrum.db")
    }

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    pub fn create_session(&self, name: String, profile: String) -> anyhow::Result<Session> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Self::now();
        self.conn.execute(
            "INSERT INTO sessions (id, name, profile, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, name, profile, now, now],
        )?;
        Ok(Session {
            id,
            name,
            profile,
            created_at: now,
            updated_at: now,
            is_pinned: false,
            message_count: 0,
            last_message_preview: None,
        })
    }

    pub fn list_sessions(&self) -> anyhow::Result<Vec<Session>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.name, s.profile, s.created_at, s.updated_at, s.is_pinned,
                    COALESCE(m.cnt, 0) AS message_count,
                    m.last_preview
             FROM sessions s
             LEFT JOIN (
                 SELECT session_id,
                        COUNT(*) AS cnt,
                        (SELECT SUBSTR(m2.content, 1, 80) FROM messages m2
                         WHERE m2.session_id = messages.session_id
                         AND m2.role = 'user'
                         ORDER BY m2.id DESC LIMIT 1) AS last_preview
                 FROM messages GROUP BY session_id
             ) m ON m.session_id = s.id
             ORDER BY s.is_pinned DESC, s.updated_at DESC",
        )?;
        let sessions = stmt.query_map([], |row| {
            Ok(Session {
                id: row.get(0)?,
                name: row.get(1)?,
                profile: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
                is_pinned: row.get::<_, i32>(5)? != 0,
                message_count: row.get::<_, i64>(6)? as usize,
                last_message_preview: row.get::<_, Option<String>>(7)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
        Ok(sessions)
    }

    pub fn save_message(&self, session_id: &str, message: &Message) -> anyhow::Result<()> {
        let now = Self::now();
        self.conn.execute(
            "INSERT INTO messages (session_id, role, content, tool_calls, tool_name, token_count, created_at, is_compacted)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                session_id,
                message.role,
                message.content,
                message.tool_calls,
                message.tool_name,
                message.token_count as i64,
                now,
                message.is_compacted as i32,
            ],
        )?;
        self.conn.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![now, session_id],
        )?;
        Ok(())
    }

    /// Import a backed-up session with its original IDs and timestamps.
    /// Returns true if the session was inserted, false if it already existed.
    pub fn import_session(&self, session: &Session, messages: &[Message]) -> anyhow::Result<bool> {
        let existing = self
            .conn
            .query_row(
                "SELECT id FROM sessions WHERE id = ?1 LIMIT 1",
                params![session.id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;

        if existing.is_some() {
            return Ok(false);
        }

        self.conn.execute(
            "INSERT INTO sessions (id, name, profile, created_at, updated_at, is_pinned)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                session.id,
                session.name,
                session.profile,
                session.created_at as i64,
                session.updated_at as i64,
                session.is_pinned as i32,
            ],
        )?;

        for message in messages {
            self.conn.execute(
                "INSERT INTO messages (session_id, role, content, tool_calls, tool_name, token_count, created_at, is_compacted)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    session.id,
                    message.role,
                    message.content,
                    message.tool_calls,
                    message.tool_name,
                    message.token_count as i64,
                    message.created_at as i64,
                    message.is_compacted as i32,
                ],
            )?;
        }

        Ok(true)
    }

    pub fn load_messages(&self, session_id: &str) -> anyhow::Result<Vec<Message>> {
        let mut stmt = self.conn.prepare(
            "SELECT role, content, tool_calls, tool_name, token_count, created_at, is_compacted
             FROM messages WHERE session_id = ?1 ORDER BY id ASC",
        )?;
        let messages = stmt.query_map(params![session_id], |row| {
            Ok(Message {
                role: row.get(0)?,
                content: row.get(1)?,
                tool_calls: row.get(2)?,
                tool_name: row.get(3)?,
                token_count: row.get::<_, i64>(4)? as usize,
                created_at: row.get(5)?,
                is_compacted: row.get::<_, i32>(6)? != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
        Ok(messages)
    }

    pub fn delete_session(&self, session_id: &str) -> anyhow::Result<()> {
        self.conn.execute("DELETE FROM messages WHERE session_id = ?1", params![session_id])?;
        self.conn.execute("DELETE FROM sessions WHERE id = ?1", params![session_id])?;
        Ok(())
    }

    pub fn rename_session(&self, session_id: &str, new_name: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE sessions SET name = ?1, updated_at = ?2 WHERE id = ?3",
            params![new_name, Self::now(), session_id],
        )?;
        Ok(())
    }

    pub fn pin_session(&self, session_id: &str, pinned: bool) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE sessions SET is_pinned = ?1 WHERE id = ?2",
            params![pinned as i32, session_id],
        )?;
        Ok(())
    }

    pub fn get_latest_session(&self) -> anyhow::Result<Option<Session>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, profile, created_at, updated_at, is_pinned FROM sessions ORDER BY updated_at DESC LIMIT 1",
        )?;
        let mut sessions = stmt.query_map([], |row| {
            Ok(Session {
                id: row.get(0)?,
                name: row.get(1)?,
                profile: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
                is_pinned: row.get::<_, i32>(5)? != 0,
                message_count: 0,
                last_message_preview: None,
            })
        })?;
        match sessions.next() {
            Some(Ok(s)) => Ok(Some(s)),
            _ => Ok(None),
        }
    }

    pub fn clear_session_messages(&self, session_id: &str) -> anyhow::Result<()> {
        self.conn.execute("DELETE FROM messages WHERE session_id = ?1", params![session_id])?;
        Ok(())
    }

    pub fn get_session_token_count(&self, session_id: &str) -> anyhow::Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(token_count), 0) FROM messages WHERE session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    pub fn get_message_count(&self, session_id: &str) -> anyhow::Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    pub fn export_session_markdown(&self, session_id: &str) -> anyhow::Result<String> {
        let mut stmt = self.conn.prepare(
            "SELECT s.name, s.profile FROM sessions s WHERE s.id = ?1",
        )?;
        let (name, profile): (String, String) = stmt.query_row(params![session_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;

        let messages = self.load_messages(session_id)?;
        let mut md = format!("# {}\n\n**Profile:** {}\n\n---\n\n", name, profile);

        for msg in &messages {
            let label = match msg.role.as_str() {
                "user" => "**You**",
                "assistant" => "**Assistant**",
                "system" => "**System**",
                "tool" => "**Tool**",
                _ => &msg.role,
            };
            md.push_str(&format!("{}\n\n{}\n\n---\n\n", label, msg.content));
        }

        Ok(md)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_message(role: &str, content: &str) -> Message {
        Message {
            role: role.to_string(),
            content: content.to_string(),
            tool_calls: None,
            tool_name: None,
            token_count: content.split_whitespace().count(),
            is_compacted: false,
            created_at: 0,
        }
    }

    #[test]
    fn create_and_list_sessions() {
        let store = SessionStore::open_in_memory().unwrap();
        let s1 = store.create_session("Test 1".into(), "local-llama".into()).unwrap();
        let s2 = store.create_session("Test 2".into(), "lm-studio".into()).unwrap();
        assert_ne!(s1.id, s2.id);

        let sessions = store.list_sessions().unwrap();
        assert_eq!(sessions.len(), 2);
    }

    #[test]
    fn save_and_load_messages() {
        let store = SessionStore::open_in_memory().unwrap();
        let session = store.create_session("Chat".into(), "default".into()).unwrap();

        store.save_message(&session.id, &test_message("user", "Hello")).unwrap();
        store.save_message(&session.id, &test_message("assistant", "Hi there!")).unwrap();

        let messages = store.load_messages(&session.id).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "Hello");
        assert_eq!(messages[1].role, "assistant");
    }

    #[test]
    fn delete_session_cascades() {
        let store = SessionStore::open_in_memory().unwrap();
        let session = store.create_session("Temp".into(), "default".into()).unwrap();
        store.save_message(&session.id, &test_message("user", "test")).unwrap();

        store.delete_session(&session.id).unwrap();
        let sessions = store.list_sessions().unwrap();
        assert!(sessions.is_empty());
        let messages = store.load_messages(&session.id).unwrap();
        assert!(messages.is_empty());
    }

    #[test]
    fn rename_session() {
        let store = SessionStore::open_in_memory().unwrap();
        let session = store.create_session("Old Name".into(), "default".into()).unwrap();
        store.rename_session(&session.id, "New Name").unwrap();

        let sessions = store.list_sessions().unwrap();
        assert_eq!(sessions[0].name, "New Name");
    }

    #[test]
    fn pin_session() {
        let store = SessionStore::open_in_memory().unwrap();
        let s1 = store.create_session("Unpinned".into(), "default".into()).unwrap();
        let s2 = store.create_session("Pinned".into(), "default".into()).unwrap();
        store.pin_session(&s2.id, true).unwrap();

        let sessions = store.list_sessions().unwrap();
        // Pinned should come first
        assert!(sessions[0].is_pinned);
        assert_eq!(sessions[0].name, "Pinned");
        assert!(!sessions[1].is_pinned);
        // Verify s1 is unpinned
        let _ = s1;
    }

    #[test]
    fn get_latest_session_empty() {
        let store = SessionStore::open_in_memory().unwrap();
        assert!(store.get_latest_session().unwrap().is_none());
    }

    #[test]
    fn get_latest_session_returns_one() {
        let store = SessionStore::open_in_memory().unwrap();
        let session = store.create_session("Only".into(), "default".into()).unwrap();
        let latest = store.get_latest_session().unwrap().unwrap();
        assert_eq!(latest.id, session.id);
    }

    #[test]
    fn clear_session_messages() {
        let store = SessionStore::open_in_memory().unwrap();
        let session = store.create_session("Chat".into(), "default".into()).unwrap();
        store.save_message(&session.id, &test_message("user", "msg1")).unwrap();
        store.save_message(&session.id, &test_message("assistant", "msg2")).unwrap();

        store.clear_session_messages(&session.id).unwrap();
        assert_eq!(store.load_messages(&session.id).unwrap().len(), 0);
        // Session itself still exists
        assert_eq!(store.list_sessions().unwrap().len(), 1);
    }

    #[test]
    fn token_count_aggregation() {
        let store = SessionStore::open_in_memory().unwrap();
        let session = store.create_session("Chat".into(), "default".into()).unwrap();

        let mut msg = test_message("user", "hello world");
        msg.token_count = 10;
        store.save_message(&session.id, &msg).unwrap();

        let mut msg2 = test_message("assistant", "hi there friend");
        msg2.token_count = 20;
        store.save_message(&session.id, &msg2).unwrap();

        assert_eq!(store.get_session_token_count(&session.id).unwrap(), 30);
    }

    #[test]
    fn message_count() {
        let store = SessionStore::open_in_memory().unwrap();
        let session = store.create_session("Chat".into(), "default".into()).unwrap();
        assert_eq!(store.get_message_count(&session.id).unwrap(), 0);

        store.save_message(&session.id, &test_message("user", "1")).unwrap();
        store.save_message(&session.id, &test_message("assistant", "2")).unwrap();
        assert_eq!(store.get_message_count(&session.id).unwrap(), 2);
    }

    #[test]
    fn export_markdown() {
        let store = SessionStore::open_in_memory().unwrap();
        let session = store.create_session("My Chat".into(), "gpt-4".into()).unwrap();
        store.save_message(&session.id, &test_message("user", "Hello")).unwrap();
        store.save_message(&session.id, &test_message("assistant", "Hi!")).unwrap();

        let md = store.export_session_markdown(&session.id).unwrap();
        assert!(md.contains("# My Chat"));
        assert!(md.contains("**Profile:** gpt-4"));
        assert!(md.contains("**You**"));
        assert!(md.contains("Hello"));
        assert!(md.contains("**Assistant**"));
        assert!(md.contains("Hi!"));
    }

    #[test]
    fn message_ordering_preserved() {
        let store = SessionStore::open_in_memory().unwrap();
        let session = store.create_session("Chat".into(), "default".into()).unwrap();

        for i in 0..10 {
            store.save_message(&session.id, &test_message("user", &format!("msg {}", i))).unwrap();
        }

        let messages = store.load_messages(&session.id).unwrap();
        for (i, msg) in messages.iter().enumerate() {
            assert_eq!(msg.content, format!("msg {}", i));
        }
    }

    #[test]
    fn import_session_skips_duplicates() {
        let store = SessionStore::open_in_memory().unwrap();
        let session = Session {
            id: "cloud-sync-session".into(),
            name: "Synced Session".into(),
            profile: "default".into(),
            created_at: 12,
            updated_at: 34,
            is_pinned: true,
            message_count: 0,
            last_message_preview: None,
        };
        let messages = vec![test_message("user", "hello from cloud")];

        assert!(store.import_session(&session, &messages).unwrap());
        assert!(!store.import_session(&session, &messages).unwrap());

        let sessions = store.list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "cloud-sync-session");
        assert_eq!(store.load_messages("cloud-sync-session").unwrap().len(), 1);
    }
}
