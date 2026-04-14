use crate::parser::Triple;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::path::Path;

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(db_path: &Path) -> Result<Store, String> {
        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create DB directory: {e}"))?;
        }

        let conn = Connection::open(db_path)
            .map_err(|e| format!("Failed to open database: {e}"))?;

        let store = Store { conn };
        store.init_schema().map_err(|e| format!("Failed to init schema: {e}"))?;
        Ok(store)
    }

    fn init_schema(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS files (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL UNIQUE,
                checksum TEXT NOT NULL DEFAULT '',
                mtime REAL DEFAULT 0,
                scanned_at TEXT
            );

            CREATE TABLE IF NOT EXISTS triples (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
                s TEXT NOT NULL,
                p TEXT NOT NULL,
                o TEXT NOT NULL,
                domain TEXT DEFAULT 'general',
                confidence REAL DEFAULT 0.7,
                line_start INTEGER,
                line_end INTEGER,
                source_file TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_triples_file ON triples(file_id);
            CREATE INDEX IF NOT EXISTS idx_triples_p ON triples(p);
            CREATE INDEX IF NOT EXISTS idx_triples_s ON triples(s);

            CREATE VIRTUAL TABLE IF NOT EXISTS triples_fts USING fts5(
                s, p, o,
                content='triples',
                content_rowid='id'
            );

            CREATE TRIGGER IF NOT EXISTS triples_ai AFTER INSERT ON triples BEGIN
                INSERT INTO triples_fts(rowid, s, p, o) VALUES (new.id, new.s, new.p, new.o);
            END;

            CREATE TRIGGER IF NOT EXISTS triples_ad AFTER DELETE ON triples BEGIN
                INSERT INTO triples_fts(triples_fts, rowid, s, p, o)
                    VALUES('delete', old.id, old.s, old.p, old.o);
            END;

            CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY,
                value TEXT
            );

            CREATE TABLE IF NOT EXISTS code_graph (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                directory TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                source_file TEXT NOT NULL,
                scanned_at TEXT,
                UNIQUE(directory, tool_name, source_file)
            );

            CREATE INDEX IF NOT EXISTS idx_code_graph_dir ON code_graph(directory);

            CREATE TABLE IF NOT EXISTS rule_intent (
                triple_id INTEGER PRIMARY KEY REFERENCES triples(id) ON DELETE CASCADE,
                action TEXT NOT NULL DEFAULT 'general',
                timing TEXT NOT NULL DEFAULT 'tool_call',
                tools TEXT NOT NULL DEFAULT '*',
                allow_inverse INTEGER DEFAULT 0,
                enriched_by TEXT DEFAULT 'taxonomy',
                enriched_at TEXT
            );

            PRAGMA foreign_keys = ON;
            ",
        )
    }

    /// Upsert a file and its triples. Skips if checksum unchanged.
    pub fn upsert_file(
        &self,
        path: &str,
        content: &str,
        triples: &[Triple],
        source_type: &str,
    ) -> rusqlite::Result<bool> {
        let checksum = compute_checksum(content);

        // Check if file exists with same checksum
        let existing_checksum: Option<String> = self
            .conn
            .query_row(
                "SELECT checksum FROM files WHERE path = ?1",
                params![path],
                |row| row.get(0),
            )
            .ok();

        if existing_checksum.as_deref() == Some(&checksum) {
            return Ok(false); // No change
        }

        let tx = self.conn.unchecked_transaction()?;

        // Get or create file_id
        tx.execute(
            "INSERT INTO files (path, checksum, scanned_at)
             VALUES (?1, ?2, datetime('now'))
             ON CONFLICT(path) DO UPDATE SET checksum = ?2, scanned_at = datetime('now')",
            params![path, checksum],
        )?;

        let file_id: i64 = tx.query_row(
            "SELECT id FROM files WHERE path = ?1",
            params![path],
            |row| row.get(0),
        )?;

        // Delete old triples for this file (triggers handle FTS cleanup)
        tx.execute("DELETE FROM triples WHERE file_id = ?1", params![file_id])?;

        // Insert new triples
        for triple in triples {
            tx.execute(
                "INSERT INTO triples (file_id, s, p, o, domain, confidence, line_start, line_end, source_file)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    file_id,
                    triple.subject,
                    triple.predicate,
                    triple.object,
                    format!("memory.{source_type}"),
                    triple.confidence,
                    triple.line_start,
                    triple.line_end,
                    path,
                ],
            )?;
        }

        tx.commit()?;
        Ok(true)
    }

    /// Load all actionable guardrails, ordered by confidence.
    pub fn load_guardrails(&self) -> rusqlite::Result<Vec<Guardrail>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.id, t.s, t.p, t.o, t.confidence, t.source_file, f.path
             FROM triples t
             JOIN files f ON t.file_id = f.id
             WHERE t.p IN ('forbids','must_not','never','always','requires','enforces','prefers')
             ORDER BY t.confidence DESC",
        )?;

        let guardrails = stmt
            .query_map([], |row| {
                Ok(Guardrail {
                    triple_id: row.get(0)?,
                    subject: row.get(1)?,
                    predicate: row.get(2)?,
                    object: row.get(3)?,
                    confidence: row.get(4)?,
                    source_file: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                    file_path: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(guardrails)
    }

    pub fn list_files(&self) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT path FROM files ORDER BY path")?;
        let paths = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(paths)
    }

    pub fn guardrail_count(&self) -> rusqlite::Result<i64> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM triples WHERE p IN ('forbids','must_not','never','always','requires','enforces','prefers')",
            [],
            |row| row.get::<_, i64>(0),
        )
    }

    pub fn get_meta(&self, key: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                _ => Err(e),
            })
    }

    pub fn set_meta(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = ?2",
            params![key, value],
        )?;
        Ok(())
    }

    // --- Code graph methods ---

    /// Bulk upsert code graph entries from a project scan.
    pub fn upsert_code_graph(
        &self,
        imports: &[crate::code_scanner::ImportInfo],
    ) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;

        // Clear existing entries
        tx.execute("DELETE FROM code_graph", [])?;

        for import in imports {
            tx.execute(
                "INSERT OR IGNORE INTO code_graph (directory, tool_name, source_file, scanned_at)
                 VALUES (?1, ?2, ?3, datetime('now'))",
                params![import.directory, import.tool_name, import.source_file],
            )?;
        }

        tx.commit()
    }

    /// Query what tools are imported by files in a given directory.
    pub fn query_tools_for_directory(&self, directory: &str) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT tool_name FROM code_graph WHERE directory = ?1",
        )?;
        let tools = stmt
            .query_map(params![directory], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(tools)
    }

    /// Query tools for a file path by checking its parent directories.
    /// Walks up from the file's directory to find tools used by sibling files.
    pub fn query_tools_for_path(&self, file_path: &str) -> rusqlite::Result<Vec<String>> {
        let path = std::path::Path::new(file_path);
        let mut tools = Vec::new();

        // Check the file's own directory and up to 2 parent levels
        let mut dir = path.parent();
        for _ in 0..3 {
            if let Some(d) = dir {
                let dir_str = d.to_string_lossy();
                let mut dir_tools = self.query_tools_for_directory(&dir_str)?;
                tools.append(&mut dir_tools);
                dir = d.parent();
            } else {
                break;
            }
        }

        tools.sort();
        tools.dedup();
        Ok(tools)
    }

    /// Get the count of unique tools in the code graph.
    pub fn code_graph_tool_count(&self) -> rusqlite::Result<i64> {
        self.conn.query_row(
            "SELECT COUNT(DISTINCT tool_name) FROM code_graph",
            [],
            |row| row.get::<_, i64>(0),
        )
    }

    /// Get the count of files scanned for the code graph.
    pub fn code_graph_file_count(&self) -> rusqlite::Result<i64> {
        self.conn.query_row(
            "SELECT COUNT(DISTINCT source_file) FROM code_graph",
            [],
            |row| row.get::<_, i64>(0),
        )
    }

    // --- Rule intent methods ---

    /// Store classified intent for a triple.
    pub fn upsert_rule_intent(
        &self,
        triple_id: i64,
        intent: &crate::intent::RuleIntent,
    ) -> rusqlite::Result<()> {
        let tools_json = serde_json::to_string(&intent.tools).unwrap_or_else(|_| "[\"*\"]".to_string());
        self.conn.execute(
            "INSERT INTO rule_intent (triple_id, action, timing, tools, allow_inverse, enriched_by, enriched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
             ON CONFLICT(triple_id) DO UPDATE SET
                action = ?2, timing = ?3, tools = ?4, allow_inverse = ?5, enriched_by = ?6, enriched_at = datetime('now')",
            params![
                triple_id,
                intent.action.as_str(),
                intent.timing.as_str(),
                tools_json,
                intent.allow_inverse as i32,
                intent.enriched_by,
            ],
        )?;
        Ok(())
    }

    /// Get the intent for a specific triple.
    pub fn get_rule_intent(&self, triple_id: i64) -> rusqlite::Result<Option<crate::intent::RuleIntent>> {
        match self.conn.query_row(
            "SELECT action, timing, tools, allow_inverse, enriched_by FROM rule_intent WHERE triple_id = ?1",
            params![triple_id],
            |row| {
                let action_str: String = row.get(0)?;
                let timing_str: String = row.get(1)?;
                let tools_json: String = row.get(2)?;
                let allow_inverse: i32 = row.get(3)?;
                let enriched_by: String = row.get(4)?;

                let tools: Vec<String> = serde_json::from_str(&tools_json)
                    .unwrap_or_else(|_| vec!["*".to_string()]);

                Ok(crate::intent::RuleIntent {
                    action: crate::intent::Action::from_str(&action_str),
                    timing: crate::intent::Timing::from_str(&timing_str),
                    tools,
                    allow_inverse: allow_inverse != 0,
                    enriched_by,
                })
            },
        ) {
            Ok(intent) => Ok(Some(intent)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Classify all existing guardrails using the taxonomy.
    pub fn classify_all_guardrails(&self) -> rusqlite::Result<usize> {
        let guardrails = self.load_guardrails()?;
        let mut count = 0;
        for g in &guardrails {
            let intent = crate::intent::classify_rule_with_subject(&g.predicate, &g.object, Some(&g.subject));
            self.upsert_rule_intent(g.triple_id, &intent)?;
            count += 1;
        }
        Ok(count)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Guardrail {
    pub triple_id: i64,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub confidence: f64,
    pub source_file: String,
    pub file_path: String,
}

fn compute_checksum(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let hash = hasher.finalize();
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use std::sync::atomic::{AtomicU64, Ordering};
    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_db() -> (Store, PathBuf) {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("arai_test_{}_{}", std::process::id(), id));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test.db");
        let store = Store::open(&db_path).unwrap();
        (store, dir)
    }

    #[test]
    fn test_upsert_and_load() {
        let (store, dir) = temp_db();

        let triples = vec![Triple {
            subject: "Git".to_string(),
            predicate: "never".to_string(),
            object: "force-push to main".to_string(),
            confidence: 0.92,
            domain: "memory.claude_md_project".to_string(),
            source_file: "CLAUDE.md".to_string(),
            line_start: Some(1),
            line_end: Some(1),
        }];

        let changed = store.upsert_file("CLAUDE.md", "- Never force-push to main", &triples, "claude_md_project").unwrap();
        assert!(changed);

        let guardrails = store.load_guardrails().unwrap();
        assert_eq!(guardrails.len(), 1);
        assert_eq!(guardrails[0].subject, "Git");
        assert_eq!(guardrails[0].predicate, "never");

        // Upsert again with same content — should skip
        let changed = store.upsert_file("CLAUDE.md", "- Never force-push to main", &triples, "claude_md_project").unwrap();
        assert!(!changed);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_meta() {
        let (store, dir) = temp_db();

        assert_eq!(store.get_meta("last_scan").unwrap(), None);
        store.set_meta("last_scan", "12345").unwrap();
        assert_eq!(store.get_meta("last_scan").unwrap(), Some("12345".to_string()));

        std::fs::remove_dir_all(&dir).ok();
    }
}
