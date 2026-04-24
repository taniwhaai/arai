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
                source_file TEXT,
                layer INTEGER,
                expires_at TEXT
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
                enriched_at TEXT,
                severity TEXT NOT NULL DEFAULT 'warn'
            );

            PRAGMA foreign_keys = ON;
            ",
        )?;

        // Migration: add severity column to older rule_intent tables.  Safe to
        // call on every open — the ALTER is a no-op once the column exists.
        let _ = self.conn.execute(
            "ALTER TABLE rule_intent ADD COLUMN severity TEXT NOT NULL DEFAULT 'warn'",
            [],
        );

        // Migration: add layer column to older triples tables so derivation
        // trace surfaces even on upgraded stores (NULL for pre-trace rows).
        let _ = self.conn.execute(
            "ALTER TABLE triples ADD COLUMN layer INTEGER",
            [],
        );

        // Migration: add expires_at for rule self-pruning.  ISO date strings;
        // NULL for rules without an annotation.
        let _ = self.conn.execute(
            "ALTER TABLE triples ADD COLUMN expires_at TEXT",
            [],
        );

        Ok(())
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
                "INSERT INTO triples (file_id, s, p, o, domain, confidence, line_start, line_end, source_file, layer, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
                    triple.layer.map(|l| l as i64),
                    triple.expires_at,
                ],
            )?;
        }

        tx.commit()?;
        Ok(true)
    }

    /// Load all actionable guardrails, ordered by confidence.  Rules with an
    /// `expires_at` date in the past are filtered out — they self-prune
    /// without any manual cleanup.
    pub fn load_guardrails(&self) -> rusqlite::Result<Vec<Guardrail>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.id, t.s, t.p, t.o, t.confidence, t.source_file, f.path, t.layer, t.line_start, t.expires_at
             FROM triples t
             JOIN files f ON t.file_id = f.id
             WHERE t.p IN ('forbids','must_not','never','always','requires','enforces','prefers')
               AND (t.expires_at IS NULL OR t.expires_at >= date('now'))
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
                    layer: row.get::<_, Option<i64>>(7)?.map(|v| v as u8),
                    line_start: row.get::<_, Option<i64>>(8)?,
                    expires_at: row.get::<_, Option<String>>(9)?,
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
            "INSERT INTO rule_intent (triple_id, action, timing, tools, allow_inverse, enriched_by, enriched_at, severity)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'), ?7)
             ON CONFLICT(triple_id) DO UPDATE SET
                action = ?2, timing = ?3, tools = ?4, allow_inverse = ?5, enriched_by = ?6, enriched_at = datetime('now'), severity = ?7",
            params![
                triple_id,
                intent.action.as_str(),
                intent.timing.as_str(),
                tools_json,
                intent.allow_inverse as i32,
                intent.enriched_by,
                intent.severity.as_str(),
            ],
        )?;
        Ok(())
    }

    /// Get the intent for a specific triple.
    pub fn get_rule_intent(&self, triple_id: i64) -> rusqlite::Result<Option<crate::intent::RuleIntent>> {
        match self.conn.query_row(
            "SELECT action, timing, tools, allow_inverse, enriched_by, severity FROM rule_intent WHERE triple_id = ?1",
            params![triple_id],
            |row| {
                let action_str: String = row.get(0)?;
                let timing_str: String = row.get(1)?;
                let tools_json: String = row.get(2)?;
                let allow_inverse: i32 = row.get(3)?;
                let enriched_by: String = row.get(4)?;
                let severity_str: String = row.get::<_, Option<String>>(5)?.unwrap_or_else(|| "warn".to_string());

                let tools: Vec<String> = serde_json::from_str(&tools_json)
                    .unwrap_or_else(|_| vec!["*".to_string()]);

                Ok(crate::intent::RuleIntent {
                    action: crate::intent::Action::from_str(&action_str),
                    timing: crate::intent::Timing::from_str(&timing_str),
                    tools,
                    allow_inverse: allow_inverse != 0,
                    enriched_by,
                    severity: crate::intent::Severity::from_str(&severity_str),
                })
            },
        ) {
            Ok(intent) => Ok(Some(intent)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Find pairs of rules that potentially conflict or duplicate.
    ///
    /// - **Duplicate**: the same (subject, predicate, object) appears in
    ///   two or more source files.  These are usually safe to consolidate
    ///   into one source to reduce CLAUDE.md drift.
    /// - **Opposing**: the same subject carries both a prohibitive
    ///   predicate (`never`, `must_not`, `avoid`) and a required predicate
    ///   (`always`, `must`, `requires`, `ensure`).  Not necessarily a real
    ///   conflict — the objects may be different — but worth a human look.
    pub fn find_rule_issues(&self) -> rusqlite::Result<RuleIssues> {
        let mut stmt = self.conn.prepare(
            "SELECT t.s, t.p, t.o, f.path FROM triples t \
             JOIN files f ON f.id = t.file_id \
             ORDER BY t.s, t.p, t.o",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;

        let mut by_spo: std::collections::BTreeMap<(String, String, String), Vec<String>> =
            std::collections::BTreeMap::new();
        let mut by_subject: std::collections::BTreeMap<String, Vec<(String, String)>> =
            std::collections::BTreeMap::new();

        for row in rows {
            let (s, p, o, src) = row?;
            by_spo
                .entry((s.clone(), p.clone(), o.clone()))
                .or_default()
                .push(src);
            by_subject.entry(s).or_default().push((p, o));
        }

        let mut duplicates = Vec::new();
        for ((s, p, o), sources) in by_spo {
            let unique: std::collections::BTreeSet<_> = sources.iter().cloned().collect();
            if unique.len() >= 2 {
                let mut list: Vec<String> = unique.into_iter().collect();
                list.sort();
                duplicates.push(DuplicateRule {
                    subject: s,
                    predicate: p,
                    object: o,
                    sources: list,
                });
            }
        }

        let mut opposing = Vec::new();
        for (subject, entries) in by_subject {
            let has_prohibitive = entries.iter().any(|(p, _)| is_prohibitive(p));
            let has_required = entries.iter().any(|(p, _)| is_required(p));
            if has_prohibitive && has_required {
                let mut preds: Vec<String> = entries.iter().map(|(p, _)| p.clone()).collect();
                preds.sort();
                preds.dedup();
                opposing.push(OpposingRules {
                    subject,
                    predicates: preds,
                });
            }
        }

        Ok(RuleIssues { duplicates, opposing })
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

/// Summary of rule-set health issues surfaced by [`Store::find_rule_issues`].
#[derive(Debug, Default, serde::Serialize)]
pub struct RuleIssues {
    pub duplicates: Vec<DuplicateRule>,
    pub opposing: Vec<OpposingRules>,
}

#[derive(Debug, serde::Serialize)]
pub struct DuplicateRule {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub sources: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct OpposingRules {
    pub subject: String,
    pub predicates: Vec<String>,
}

fn is_prohibitive(p: &str) -> bool {
    matches!(
        p,
        "never" | "must_not" | "avoid" | "do_not" | "dont" | "forbid"
    )
}

fn is_required(p: &str) -> bool {
    matches!(
        p,
        "always" | "must" | "requires" | "ensure" | "prefer" | "should"
    )
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
    /// Parser layer (1..=6) that produced this rule.  Preserved through the
    /// store so `arai audit` / `arai why` can trace a firing back to the
    /// exact regex family that extracted the rule.  `None` for rules
    /// ingested before the layer column existed, or rules added manually.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<u8>,
    /// Line number in the source file where the rule was extracted.  Surfaced
    /// alongside the layer so the audit trace reads like "CLAUDE.md:42
    /// (layer-1 imperative)".  `None` for manually-added rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_start: Option<i64>,
    /// Optional ISO date (`YYYY-MM-DD`) after which this rule is filtered
    /// out of `load_guardrails`.  Extracted at parse time from a trailing
    /// `(expires ...)` annotation; always `None` for rules without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
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
            layer: None,
            expires_at: None,
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
    fn test_find_rule_issues_detects_duplicates() {
        let (store, dir) = temp_db();

        let t = Triple {
            subject: "git".to_string(),
            predicate: "never".to_string(),
            object: "force-push to main".to_string(),
            confidence: 0.92,
            domain: "general".to_string(),
            source_file: "CLAUDE.md".to_string(),
            line_start: Some(1),
            line_end: Some(1),
            layer: None,
            expires_at: None,
        };
        store.upsert_file("CLAUDE.md", "- a", &[t.clone()], "claude_md_project").unwrap();
        store.upsert_file(
            "global.md",
            "- a",
            &[Triple { source_file: "global.md".to_string(), ..t.clone() }],
            "claude_md_global",
        ).unwrap();

        let issues = store.find_rule_issues().unwrap();
        assert_eq!(issues.duplicates.len(), 1);
        assert_eq!(issues.duplicates[0].subject, "git");
        assert_eq!(issues.duplicates[0].sources.len(), 2);
        assert!(issues.opposing.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_find_rule_issues_detects_opposing() {
        let (store, dir) = temp_db();

        let never = Triple {
            subject: "alembic".to_string(),
            predicate: "never".to_string(),
            object: "hand-write migrations".to_string(),
            confidence: 0.9,
            domain: "general".to_string(),
            source_file: "CLAUDE.md".to_string(),
            line_start: Some(1),
            line_end: Some(1),
            layer: None,
            expires_at: None,
        };
        let always = Triple {
            subject: "alembic".to_string(),
            predicate: "always".to_string(),
            object: "hand-write migrations".to_string(),
            confidence: 0.9,
            domain: "general".to_string(),
            source_file: "CLAUDE.md".to_string(),
            line_start: Some(2),
            line_end: Some(2),
            layer: None,
            expires_at: None,
        };
        store.upsert_file("CLAUDE.md", "- a\n- b", &[never, always], "claude_md_project").unwrap();

        let issues = store.find_rule_issues().unwrap();
        assert_eq!(issues.opposing.len(), 1);
        assert_eq!(issues.opposing[0].subject, "alembic");
        assert!(issues.opposing[0].predicates.contains(&"never".to_string()));
        assert!(issues.opposing[0].predicates.contains(&"always".to_string()));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_find_rule_issues_clean_set_is_empty() {
        let (store, dir) = temp_db();

        let t1 = Triple {
            subject: "git".to_string(),
            predicate: "never".to_string(),
            object: "force-push".to_string(),
            confidence: 0.9,
            domain: "general".to_string(),
            source_file: "CLAUDE.md".to_string(),
            line_start: Some(1),
            line_end: Some(1),
            layer: None,
            expires_at: None,
        };
        let t2 = Triple {
            subject: "alembic".to_string(),
            predicate: "never".to_string(),
            object: "hand-write".to_string(),
            confidence: 0.9,
            domain: "general".to_string(),
            source_file: "CLAUDE.md".to_string(),
            line_start: Some(2),
            line_end: Some(2),
            layer: None,
            expires_at: None,
        };
        store.upsert_file("CLAUDE.md", "- a\n- b", &[t1, t2], "claude_md_project").unwrap();

        let issues = store.find_rule_issues().unwrap();
        assert!(issues.duplicates.is_empty());
        assert!(issues.opposing.is_empty());

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

    #[test]
    fn test_expired_rules_are_filtered_on_load() {
        let (store, dir) = temp_db();
        let alive = Triple {
            subject: "git".to_string(),
            predicate: "never".to_string(),
            object: "force-push to main".to_string(),
            confidence: 0.92,
            domain: "general".to_string(),
            source_file: "CLAUDE.md".to_string(),
            line_start: Some(1),
            line_end: Some(1),
            layer: Some(1),
            expires_at: Some("2099-01-01".to_string()), // far future
        };
        let dead = Triple {
            subject: "legacy".to_string(),
            predicate: "never".to_string(),
            object: "touch the old api".to_string(),
            confidence: 0.92,
            domain: "general".to_string(),
            source_file: "CLAUDE.md".to_string(),
            line_start: Some(2),
            line_end: Some(2),
            layer: Some(1),
            expires_at: Some("2000-01-01".to_string()), // long past
        };
        store.upsert_file("CLAUDE.md", "x", &[alive, dead], "claude_md_project").unwrap();

        let rails = store.load_guardrails().unwrap();
        let subjects: Vec<_> = rails.iter().map(|g| g.subject.clone()).collect();
        assert!(subjects.iter().any(|s| s == "git"), "unexpired rule should load");
        assert!(!subjects.iter().any(|s| s == "legacy"), "expired rule should NOT load");

        std::fs::remove_dir_all(&dir).ok();
    }
}
