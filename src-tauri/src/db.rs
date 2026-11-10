use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub id: i64,
    pub kind: String, // "text" | "image"
    /// 表示用プレビュー（長文は切り詰め）
    pub preview: String,
    pub char_count: i64,
    pub image_path: Option<String>,
    pub thumb_path: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub source_name: Option<String>,
    pub source_id: Option<String>,
    pub source_icon: Option<String>,
    pub created_at: i64,
    pub used_at: i64,
    pub favorite: bool,
    pub snippet: bool,
    pub title: Option<String>,
    /// 翻訳済みならその訳文
    pub translated: Option<String>,
    pub tags: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    pub id: i64,
    pub name: String,
    pub color: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceApp {
    pub source_id: String,
    pub source_name: String,
    pub source_icon: Option<String>,
    pub count: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Filter {
    /// "all" | "favorite" | "snippet" | "tag"
    pub tab: String,
    pub tag_id: Option<i64>,
    pub source_id: Option<String>,
    pub query: Option<String>,
    pub limit: Option<i64>,
    /// この時刻(ms)以降に使った/コピーしたもの
    pub since: Option<i64>,
    /// この時刻(ms)より前
    pub until: Option<i64>,
}

pub struct NewItem<'a> {
    pub kind: &'a str,
    pub text: Option<&'a str>,
    pub image_path: Option<&'a str>,
    pub thumb_path: Option<&'a str>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub hash: &'a str,
    pub source_name: Option<&'a str>,
    pub source_id: Option<&'a str>,
    pub source_icon: Option<&'a str>,
}

pub struct Db {
    conn: Connection,
}

const PREVIEW_CHARS: usize = 400;

impl Db {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;
            CREATE TABLE IF NOT EXISTS items (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL,
                text TEXT,
                image_path TEXT,
                thumb_path TEXT,
                width INTEGER,
                height INTEGER,
                hash TEXT NOT NULL UNIQUE,
                source_name TEXT,
                source_id TEXT,
                source_icon TEXT,
                created_at INTEGER NOT NULL,
                used_at INTEGER NOT NULL,
                favorite INTEGER NOT NULL DEFAULT 0,
                snippet INTEGER NOT NULL DEFAULT 0,
                title TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_items_used ON items(used_at DESC);
            CREATE TABLE IF NOT EXISTS tags (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                color TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS item_tags (
                item_id INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
                tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
                PRIMARY KEY (item_id, tag_id)
            );
            "#,
        )?;
        // v0.1 以降に追加した列（既にあればエラーになるので無視）
        let _ = conn.execute("ALTER TABLE items ADD COLUMN translated TEXT", []);
        Ok(Self { conn })
    }

    /// 新規追加。同じ内容が既にあれば先頭へ移動（コピー元は最新で上書き）して false を返す
    pub fn insert_or_bump(&self, it: &NewItem) -> rusqlite::Result<(i64, bool)> {
        let now = now_ms();
        let existing: Option<i64> = self
            .conn
            .query_row("SELECT id FROM items WHERE hash = ?1", [it.hash], |r| r.get(0))
            .optional()?;
        if let Some(id) = existing {
            self.conn.execute(
                "UPDATE items SET used_at = ?1,
                    source_name = COALESCE(?2, source_name),
                    source_id = COALESCE(?3, source_id),
                    source_icon = COALESCE(?4, source_icon)
                 WHERE id = ?5",
                params![now, it.source_name, it.source_id, it.source_icon, id],
            )?;
            return Ok((id, false));
        }
        self.conn.execute(
            "INSERT INTO items (kind, text, image_path, thumb_path, width, height, hash,
                source_name, source_id, source_icon, created_at, used_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
            params![
                it.kind, it.text, it.image_path, it.thumb_path, it.width, it.height, it.hash,
                it.source_name, it.source_id, it.source_icon, now
            ],
        )?;
        Ok((self.conn.last_insert_rowid(), true))
    }

    fn row_to_item(r: &Row) -> rusqlite::Result<Item> {
        let text: Option<String> = r.get("text")?;
        let (preview, char_count) = match &text {
            Some(t) => {
                let count = t.chars().count();
                (t.chars().take(PREVIEW_CHARS).collect(), count as i64)
            }
            None => (String::new(), 0),
        };
        Ok(Item {
            id: r.get("id")?,
            kind: r.get("kind")?,
            preview,
            char_count,
            image_path: r.get("image_path")?,
            thumb_path: r.get("thumb_path")?,
            width: r.get("width")?,
            height: r.get("height")?,
            source_name: r.get("source_name")?,
            source_id: r.get("source_id")?,
            source_icon: r.get("source_icon")?,
            created_at: r.get("created_at")?,
            used_at: r.get("used_at")?,
            favorite: r.get::<_, i64>("favorite")? != 0,
            snippet: r.get::<_, i64>("snippet")? != 0,
            title: r.get("title")?,
            translated: r.get("translated")?,
            tags: vec![],
        })
    }

    pub fn list(&self, f: &Filter) -> rusqlite::Result<Vec<Item>> {
        let mut sql = String::from("SELECT * FROM items i WHERE 1=1");
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![];
        match f.tab.as_str() {
            "favorite" => sql.push_str(" AND favorite = 1"),
            "snippet" => sql.push_str(" AND snippet = 1"),
            "tag" => {
                sql.push_str(" AND EXISTS (SELECT 1 FROM item_tags t WHERE t.item_id = i.id AND t.tag_id = ?)");
                args.push(Box::new(f.tag_id.unwrap_or(-1)));
            }
            _ => sql.push_str(" AND snippet = 0"),
        }
        if let Some(since) = f.since {
            sql.push_str(" AND used_at >= ?");
            args.push(Box::new(since));
        }
        if let Some(until) = f.until {
            sql.push_str(" AND used_at < ?");
            args.push(Box::new(until));
        }
        if let Some(src) = f.source_id.as_ref().filter(|s| !s.is_empty()) {
            sql.push_str(" AND source_id = ?");
            args.push(Box::new(src.clone()));
        }
        if let Some(q) = f.query.as_ref().map(|q| q.trim()).filter(|q| !q.is_empty()) {
            for word in q.split_whitespace() {
                sql.push_str(" AND (text LIKE ? ESCAPE '\\' OR translated LIKE ? ESCAPE '\\' OR title LIKE ? ESCAPE '\\' OR source_name LIKE ? ESCAPE '\\')");
                let esc = word.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
                let pat = format!("%{esc}%");
                for _ in 0..3 {
                    args.push(Box::new(pat.clone()));
                }
                args.push(Box::new(pat));
            }
        }
        if f.tab == "snippet" {
            sql.push_str(" ORDER BY COALESCE(title, text) COLLATE NOCASE");
        } else {
            sql.push_str(" ORDER BY used_at DESC");
        }
        sql.push_str(" LIMIT ?");
        args.push(Box::new(f.limit.unwrap_or(300)));

        let mut stmt = self.conn.prepare(&sql)?;
        let mut items: Vec<Item> = stmt
            .query_map(rusqlite::params_from_iter(args.iter().map(|b| b.as_ref())), Self::row_to_item)?
            .collect::<Result<_, _>>()?;

        let mut tag_stmt = self.conn.prepare_cached("SELECT tag_id FROM item_tags WHERE item_id = ?1")?;
        for it in items.iter_mut() {
            it.tags = tag_stmt
                .query_map([it.id], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
        }
        Ok(items)
    }

    /// 元のテキスト（翻訳前）
    pub fn original_text(&self, id: i64) -> rusqlite::Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT text FROM items WHERE id = ?1", [id], |r| r.get(0))
            .optional()?
            .flatten())
    }

    pub fn set_translation(&self, id: i64, translated: Option<&str>) -> rusqlite::Result<()> {
        self.conn
            .execute("UPDATE items SET translated = ?1 WHERE id = ?2", params![translated, id])?;
        Ok(())
    }

    /// 貼り付け用の中身 (kind, text, image_path)。翻訳済みなら訳文を返す
    pub fn content(&self, id: i64) -> rusqlite::Result<Option<(String, Option<String>, Option<String>)>> {
        self.conn
            .query_row(
                "SELECT kind, COALESCE(translated, text), image_path FROM items WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
    }

    pub fn touch(&self, id: i64) -> rusqlite::Result<()> {
        self.conn
            .execute("UPDATE items SET used_at = ?1 WHERE id = ?2 AND snippet = 0", params![now_ms(), id])?;
        Ok(())
    }

    pub fn set_favorite(&self, id: i64, fav: bool) -> rusqlite::Result<()> {
        self.conn
            .execute("UPDATE items SET favorite = ?1 WHERE id = ?2", params![fav as i64, id])?;
        Ok(())
    }

    /// 削除して、消すべき画像ファイルのパスを返す
    pub fn delete(&self, id: i64) -> rusqlite::Result<Vec<String>> {
        let files = self.files_of(&format!("id = {id}"))?;
        self.conn.execute("DELETE FROM items WHERE id = ?1", [id])?;
        Ok(files)
    }

    fn files_of(&self, cond: &str) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT image_path, thumb_path FROM items WHERE {cond}"))?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?))
        })?;
        let mut out = vec![];
        for row in rows {
            let (a, b) = row?;
            out.extend(a);
            out.extend(b);
        }
        Ok(out)
    }

    const PRUNABLE: &'static str = "favorite = 0 AND snippet = 0";

    /// 上限を超えた古い履歴を削除（お気に入り・定型文は対象外）
    pub fn prune(&self, max: u32) -> rusqlite::Result<Vec<String>> {
        let cond = format!(
            "id IN (SELECT id FROM items WHERE {} ORDER BY used_at DESC LIMIT -1 OFFSET {})",
            Self::PRUNABLE,
            max
        );
        let files = self.files_of(&cond)?;
        self.conn.execute(&format!("DELETE FROM items WHERE {cond}"), [])?;
        Ok(files)
    }

    pub fn clear_history(&self) -> rusqlite::Result<Vec<String>> {
        let files = self.files_of(Self::PRUNABLE)?;
        self.conn
            .execute(&format!("DELETE FROM items WHERE {}", Self::PRUNABLE), [])?;
        Ok(files)
    }

    pub fn save_snippet(&self, id: Option<i64>, title: &str, text: &str) -> rusqlite::Result<i64> {
        let now = now_ms();
        let title = Some(title.trim()).filter(|t| !t.is_empty());
        if let Some(id) = id {
            self.conn.execute(
                "UPDATE items SET title = ?1, text = ?2 WHERE id = ?3 AND snippet = 1",
                params![title, text, id],
            )?;
            return Ok(id);
        }
        self.conn.execute(
            "INSERT INTO items (kind, text, hash, source_name, source_id, created_at, used_at, snippet, title)
             VALUES ('text', ?1, ?2, '定型文', 'clipo.snippet', ?3, ?3, 1, ?4)",
            params![text, format!("snippet:{now}:{}", text.len()), now, title],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_tags(&self) -> rusqlite::Result<Vec<Tag>> {
        let mut stmt = self.conn.prepare("SELECT id, name, color FROM tags ORDER BY id")?;
        stmt.query_map([], |r| Ok(Tag { id: r.get(0)?, name: r.get(1)?, color: r.get(2)? }))?
            .collect()
    }

    pub fn create_tag(&self, name: &str, color: &str) -> rusqlite::Result<Tag> {
        self.conn.execute(
            "INSERT OR IGNORE INTO tags (name, color) VALUES (?1, ?2)",
            params![name.trim(), color],
        )?;
        self.conn.query_row(
            "SELECT id, name, color FROM tags WHERE name = ?1",
            [name.trim()],
            |r| Ok(Tag { id: r.get(0)?, name: r.get(1)?, color: r.get(2)? }),
        )
    }

    pub fn delete_tag(&self, id: i64) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM tags WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn toggle_tag(&self, item_id: i64, tag_id: i64) -> rusqlite::Result<bool> {
        let removed = self.conn.execute(
            "DELETE FROM item_tags WHERE item_id = ?1 AND tag_id = ?2",
            params![item_id, tag_id],
        )?;
        if removed == 0 {
            self.conn.execute(
                "INSERT INTO item_tags (item_id, tag_id) VALUES (?1, ?2)",
                params![item_id, tag_id],
            )?;
            return Ok(true);
        }
        Ok(false)
    }

    /// タグを付ける(on=true)/外す(on=false)。何度呼んでも同じ結果になる
    pub fn set_tag(&self, item_id: i64, tag_id: i64, on: bool) -> rusqlite::Result<()> {
        if on {
            self.conn.execute(
                "INSERT OR IGNORE INTO item_tags (item_id, tag_id) VALUES (?1, ?2)",
                params![item_id, tag_id],
            )?;
        } else {
            self.conn.execute(
                "DELETE FROM item_tags WHERE item_id = ?1 AND tag_id = ?2",
                params![item_id, tag_id],
            )?;
        }
        Ok(())
    }

    pub fn source_apps(&self) -> rusqlite::Result<Vec<SourceApp>> {
        let mut stmt = self.conn.prepare(
            "SELECT source_id, MAX(source_name), MAX(source_icon), COUNT(*) FROM items
             WHERE source_id IS NOT NULL AND snippet = 0
             GROUP BY source_id ORDER BY COUNT(*) DESC",
        )?;
        stmt.query_map([], |r| {
            Ok(SourceApp {
                source_id: r.get(0)?,
                source_name: r.get(1)?,
                source_icon: r.get(2)?,
                count: r.get(3)?,
            })
        })?
        .collect()
    }
}
