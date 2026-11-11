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
    pub kind: String, // "text" | "image" | "file"
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
    /// 「保存」に入っているか（履歴の上限で消えず、履歴一覧には出ない）
    pub saved: bool,
    pub title: Option<String>,
    /// 呼び出し用の短い名前（検索でいちばん上に出る）
    pub keyword: Option<String>,
    pub use_count: i64,
    /// ファイルカードで、元のファイルが見つからないものがある
    pub missing: bool,
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
    /// "history" | "saved"
    pub tab: String,
    /// 保存タブをタグで絞り込む
    pub tag_id: Option<i64>,
    /// 種類: "text" | "link" | "image" | "file"
    pub kind: Option<String>,
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

pub fn fnv1a(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}{:x}", bytes.len())
}

/// URL だけのテキスト（リンク）
const LINK_COND: &str = "(kind = 'text' AND (trim(text) LIKE 'http://_%' OR trim(text) LIKE 'https://_%') AND instr(trim(text), ' ') = 0 \
     AND instr(trim(text), char(10)) = 0 AND instr(trim(text), char(9)) = 0)";

/// 検索欄の /画像 などを種類に読み替える
fn kind_word(w: &str) -> Option<&'static str> {
    let k = w.strip_prefix(['/', '／'])?.to_lowercase();
    Some(match k.as_str() {
        "テキスト" | "text" | "文字" => "text",
        "リンク" | "link" | "url" => "link",
        "画像" | "image" | "img" => "image",
        "ファイル" | "file" => "file",
        _ => return None,
    })
}

/// 書き出し・読み込み用の保存アイテム（テキストとリンクだけ）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportItem {
    pub text: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub keyword: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub use_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportFile {
    pub app: String,
    pub version: u32,
    pub tags: Vec<Tag>,
    pub items: Vec<ExportItem>,
}

pub fn text_hash(text: &str) -> String {
    format!("t:{}", fnv1a(text.as_bytes()))
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
        let _ = conn.execute("ALTER TABLE items ADD COLUMN keyword TEXT", []);
        let _ = conn.execute("ALTER TABLE items ADD COLUMN use_count INTEGER NOT NULL DEFAULT 0", []);
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// v0.2: お気に入り・定型文・タグ付きを「保存」(favorite 列) にまとめる
    fn migrate(&self) -> rusqlite::Result<()> {
        let ver: i64 = self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if ver >= 1 {
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE items SET favorite = 1
             WHERE snippet = 1 OR id IN (SELECT item_id FROM item_tags)",
            [],
        )?;
        // 定型文の hash を本文から作り直し、コピーと同じ内容として見分けられるようにする
        let rows: Vec<(i64, String)> = {
            let mut stmt = tx.prepare("SELECT id, text FROM items WHERE hash LIKE 'snippet:%' AND text IS NOT NULL")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<Result<_, _>>()?
        };
        for (id, text) in rows {
            let h = text_hash(&text);
            tx.execute("DELETE FROM items WHERE hash = ?1 AND favorite = 0", [&h])?;
            // 同じ本文が別に保存済みなら、古い hash のまま残す
            let _ = tx.execute("UPDATE items SET hash = ?1 WHERE id = ?2", params![h, id]);
        }
        tx.execute("PRAGMA user_version = 1", [])?;
        tx.commit()
    }

    /// 新規追加。同じ内容が既にあれば先頭へ移動（コピー元は最新で上書き）して false を返す
    pub fn insert_or_bump(&self, it: &NewItem) -> rusqlite::Result<(i64, bool)> {
        let now = now_ms();
        // 保存済みと同じ内容は履歴に出さない（保存側もそのまま）
        if let Some(t) = it.text {
            if self.is_saved_text(t)? {
                return Ok((0, false));
            }
        }
        let existing: Option<(i64, bool)> = self
            .conn
            .query_row("SELECT id, favorite FROM items WHERE hash = ?1", [it.hash], |r| {
                Ok((r.get(0)?, r.get::<_, i64>(1)? != 0))
            })
            .optional()?;
        if let Some((id, true)) = existing {
            return Ok((id, false));
        }
        if let Some((id, _)) = existing {
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
                // ファイルカードはパスの一覧なので切り詰めない（個数を正しく出すため）
                let kind: String = r.get("kind")?;
                let preview = if kind == "file" { t.clone() } else { t.chars().take(PREVIEW_CHARS).collect() };
                (preview, count as i64)
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
            saved: r.get::<_, i64>("favorite")? != 0,
            title: r.get("title")?,
            keyword: r.get("keyword")?,
            use_count: r.get("use_count")?,
            missing: false,
            translated: r.get("translated")?,
            tags: vec![],
        })
    }

    pub fn list(&self, f: &Filter) -> rusqlite::Result<Vec<Item>> {
        let mut sql = String::from("SELECT * FROM items i WHERE 1=1");
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![];
        let saved = f.tab == "saved";
        if saved {
            sql.push_str(" AND favorite = 1");
            if let Some(tag) = f.tag_id {
                sql.push_str(" AND EXISTS (SELECT 1 FROM item_tags t WHERE t.item_id = i.id AND t.tag_id = ?)");
                args.push(Box::new(tag));
            }
        } else {
            sql.push_str(" AND favorite = 0");
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
        // 種類（ボタン or 検索欄の /画像 など）
        let mut kinds: Vec<String> = f.kind.iter().cloned().collect();
        if let Some(q) = f.query.as_deref() {
            kinds.extend(q.split_whitespace().filter_map(|w| kind_word(w)).map(String::from));
        }
        for k in kinds {
            match k.as_str() {
                "link" => sql.push_str(&format!(" AND {LINK_COND}")),
                "text" => sql.push_str(&format!(" AND kind = 'text' AND NOT {LINK_COND}")),
                "image" => sql.push_str(" AND kind = 'image'"),
                "file" => sql.push_str(" AND kind = 'file'"),
                _ => {}
            }
        }
        let query = f.query.as_ref().map(|q| q.trim()).filter(|q| !q.is_empty());
        if let Some(q) = query {
            for word in q.split_whitespace() {
                // #名前 はタグの名前（前方一致）で絞り込む
                if let Some(tag) = word.strip_prefix(['#', '＃']).filter(|t| !t.is_empty()) {
                    let esc = tag.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
                    sql.push_str(" AND EXISTS (SELECT 1 FROM item_tags it JOIN tags tg ON tg.id = it.tag_id WHERE it.item_id = i.id AND tg.name LIKE ? ESCAPE '\\')");
                    args.push(Box::new(format!("{esc}%")));
                    continue;
                }
                if word == "#" || word == "＃" || word.starts_with(['/', '／']) {
                    continue;
                }
                sql.push_str(" AND (text LIKE ? ESCAPE '\\' OR translated LIKE ? ESCAPE '\\' OR title LIKE ? ESCAPE '\\' OR keyword LIKE ? ESCAPE '\\' OR source_name LIKE ? ESCAPE '\\')");
                let esc = word.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
                let pat = format!("%{esc}%");
                for _ in 0..5 {
                    args.push(Box::new(pat.clone()));
                }
            }
        }
        sql.push_str(" ORDER BY ");
        if let Some(q) = query {
            // 短い名前がぴったり一致したもの → 前方一致したもの を先頭に
            let esc = q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
            sql.push_str("(keyword = ? COLLATE NOCASE) DESC, (keyword LIKE ? ESCAPE '\\') DESC, ");
            args.push(Box::new(q.to_string()));
            args.push(Box::new(format!("{esc}%")));
        }
        sql.push_str(if saved { "use_count DESC, used_at DESC" } else { "used_at DESC" });
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

    pub fn is_saved(&self, id: i64) -> rusqlite::Result<bool> {
        Ok(self
            .conn
            .query_row("SELECT favorite FROM items WHERE id = ?1", [id], |r| r.get::<_, i64>(0))
            .optional()?
            == Some(1))
    }

    /// 貼り付けた: 履歴は先頭へ、保存は使用回数を増やす
    pub fn touch(&self, id: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE items SET used_at = ?1, use_count = use_count + 1 WHERE id = ?2",
            params![now_ms(), id],
        )?;
        Ok(())
    }

    /// 保存する / 保存を外す（外したものは履歴の先頭に戻す）
    pub fn set_saved(&self, id: i64, saved: bool) -> rusqlite::Result<()> {
        if saved {
            self.conn.execute("UPDATE items SET favorite = 1 WHERE id = ?1", [id])?;
        } else {
            self.conn
                .execute("UPDATE items SET favorite = 0, used_at = ?1 WHERE id = ?2", params![now_ms(), id])?;
        }
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

    const PRUNABLE: &'static str = "favorite = 0";

    /// 上限を超えた古い履歴を削除（保存したものは対象外）
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

    /// 保存済みのテキストと同じ内容か
    pub fn is_saved_text(&self, text: &str) -> rusqlite::Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM items WHERE favorite = 1 AND kind = 'text' AND text = ?1 LIMIT 1",
                [text],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// 保存の追加・編集。id があればそのカード（履歴でも）を保存に入れて書き換える。
    /// text は画像なら None。同じ本文が別に保存済みなら Ok(None)
    pub fn save_item(
        &self,
        id: Option<i64>,
        title: &str,
        keyword: &str,
        text: Option<&str>,
    ) -> rusqlite::Result<Option<i64>> {
        let now = now_ms();
        let norm = |s: &str| Some(s.trim().to_string()).filter(|t| !t.is_empty());
        let (title, keyword) = (norm(title), norm(keyword));
        let Some(text) = text else {
            let Some(id) = id else { return Ok(None) };
            self.conn.execute(
                "UPDATE items SET favorite = 1, title = ?1, keyword = ?2 WHERE id = ?3",
                params![title, keyword, id],
            )?;
            return Ok(Some(id));
        };
        let hash = text_hash(text);
        let dup: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM items WHERE favorite = 1 AND (hash = ?1 OR (kind = 'text' AND text = ?2)) AND id IS NOT ?3",
                params![hash, text, id],
                |r| r.get(0),
            )
            .optional()?;
        if dup.is_some() {
            return Ok(None);
        }
        // 同じ内容の履歴は保存と重なるので消す
        self.conn.execute(
            "DELETE FROM items WHERE hash = ?1 AND favorite = 0 AND id IS NOT ?2",
            params![hash, id],
        )?;
        if let Some(id) = id {
            self.conn.execute(
                "UPDATE items SET favorite = 1, title = ?1, keyword = ?2, hash = ?3,
                    translated = CASE WHEN text = ?4 THEN translated ELSE NULL END,
                    text = ?4
                 WHERE id = ?5",
                params![title, keyword, hash, text, id],
            )?;
            return Ok(Some(id));
        }
        self.conn.execute(
            "INSERT INTO items (kind, text, hash, source_name, source_id, created_at, used_at, favorite, title, keyword)
             VALUES ('text', ?1, ?2, '自分で作成', 'clipo.saved', ?3, ?3, 1, ?4, ?5)",
            params![text, hash, now, title, keyword],
        )?;
        Ok(Some(self.conn.last_insert_rowid()))
    }

    /// 保存したテキストを書き出す（画像・ファイルは端末ごとに違うので含めない）
    pub fn export_saved(&self) -> rusqlite::Result<ExportFile> {
        let mut stmt = self.conn.prepare(
            "SELECT id, text, title, keyword, use_count FROM items
             WHERE favorite = 1 AND kind = 'text' AND text IS NOT NULL ORDER BY use_count DESC, used_at DESC",
        )?;
        let rows: Vec<(i64, ExportItem)> = stmt
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    ExportItem { text: r.get(1)?, title: r.get(2)?, keyword: r.get(3)?, tags: vec![], use_count: r.get(4)? },
                ))
            })?
            .collect::<Result<_, _>>()?;
        let mut tag_stmt = self
            .conn
            .prepare("SELECT t.name FROM item_tags it JOIN tags t ON t.id = it.tag_id WHERE it.item_id = ?1")?;
        let mut items = vec![];
        for (id, mut it) in rows {
            it.tags = tag_stmt.query_map([id], |r| r.get(0))?.collect::<Result<_, _>>()?;
            items.push(it);
        }
        Ok(ExportFile { app: "Clipo".into(), version: 1, tags: self.list_tags()?, items })
    }

    /// 読み込む。同じ本文がすでに保存済みならタイトルなどを補うだけ。(追加, 更新) の件数を返す
    pub fn import_saved(&self, f: &ExportFile) -> rusqlite::Result<(usize, usize)> {
        let tx = self.conn.unchecked_transaction()?;
        for t in &f.tags {
            tx.execute("INSERT OR IGNORE INTO tags (name, color) VALUES (?1, ?2)", params![t.name.trim(), t.color])?;
        }
        let (mut added, mut updated) = (0, 0);
        for it in &f.items {
            if it.text.is_empty() {
                continue;
            }
            let hash = text_hash(&it.text);
            let existing: Option<i64> = tx
                .query_row(
                    "SELECT id FROM items WHERE hash = ?1 OR (kind = 'text' AND text = ?2)",
                    params![hash, it.text],
                    |r| r.get(0),
                )
                .optional()?;
            let id = if let Some(id) = existing {
                tx.execute(
                    "UPDATE items SET favorite = 1, title = COALESCE(title, ?1), keyword = COALESCE(keyword, ?2),
                        use_count = MAX(use_count, ?3) WHERE id = ?4",
                    params![it.title, it.keyword, it.use_count, id],
                )?;
                updated += 1;
                id
            } else {
                let now = now_ms();
                tx.execute(
                    "INSERT INTO items (kind, text, hash, source_name, source_id, created_at, used_at, favorite, title, keyword, use_count)
                     VALUES ('text', ?1, ?2, '読み込み', 'clipo.saved', ?3, ?3, 1, ?4, ?5, ?6)",
                    params![it.text, hash, now, it.title, it.keyword, it.use_count],
                )?;
                added += 1;
                tx.last_insert_rowid()
            };
            for name in &it.tags {
                tx.execute(
                    "INSERT OR IGNORE INTO item_tags (item_id, tag_id) SELECT ?1, id FROM tags WHERE name = ?2",
                    params![id, name.trim()],
                )?;
            }
        }
        tx.commit()?;
        Ok((added, updated))
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

    /// 名前・色を変更。同じ名前のタグが既にあればエラー
    pub fn update_tag(&self, id: i64, name: &str, color: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE tags SET name = ?1, color = ?2 WHERE id = ?3",
            params![name.trim(), color, id],
        )?;
        Ok(())
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
            self.conn.execute("UPDATE items SET favorite = 1 WHERE id = ?1", [item_id])?;
            self.conn.execute(
                "INSERT INTO item_tags (item_id, tag_id) VALUES (?1, ?2)",
                params![item_id, tag_id],
            )?;
            return Ok(true);
        }
        Ok(false)
    }

    /// タグを付ける(on=true)/外す(on=false)。何度呼んでも同じ結果になる。
    /// タグは保存の中の分類なので、付けたカードは保存に入る
    pub fn set_tag(&self, item_id: i64, tag_id: i64, on: bool) -> rusqlite::Result<()> {
        if on {
            self.conn.execute("UPDATE items SET favorite = 1 WHERE id = ?1", [item_id])?;
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
             WHERE source_id IS NOT NULL AND favorite = 0
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
