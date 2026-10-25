//! The library database: the list of workspaces, and the tags on their images.
//!
//! It is a single SQLite file in the app's data folder (on Windows
//! `%APPDATA%\com.kaigulliksen.reflibrary`, on macOS
//! `~/Library/Application Support/com.kaigulliksen.reflibrary`, on Linux
//! `~/.local/share/com.kaigulliksen.reflibrary`).
//! Only the library's own information lives here; image files are never
//! moved, copied or changed.
//!
//! The file is the same on every system: image locations inside a workspace are
//! stored with `/` between folders, whatever the system itself uses.

use crate::platform::PATHS_IGNORE_CASE;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Schema changes, applied in order. The database remembers how many it has
/// applied (`user_version`), so new entries only ever get added to the end.
const MIGRATIONS: &[&str] = &[
    // 1: workspaces
    "CREATE TABLE workspaces (
        id         INTEGER PRIMARY KEY,
        name       TEXT    NOT NULL,
        path       TEXT    NOT NULL,
        recursive  INTEGER NOT NULL DEFAULT 0,
        position   INTEGER NOT NULL,
        created_at INTEGER NOT NULL
    );",
    // 2: tags. Images are identified by their path inside the workspace folder, so tags
    // survive pointing the workspace at a moved copy of the folder. Tags belong to one
    // workspace, and everything is removed with it.
    "CREATE TABLE images (
        id           INTEGER PRIMARY KEY,
        workspace_id INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
        rel_path     TEXT    NOT NULL COLLATE NOCASE,
        UNIQUE (workspace_id, rel_path)
    );
    CREATE TABLE tags (
        id           INTEGER PRIMARY KEY,
        workspace_id INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
        name         TEXT    NOT NULL COLLATE NOCASE,
        created_at   INTEGER NOT NULL,
        UNIQUE (workspace_id, name)
    );
    CREATE TABLE image_tags (
        image_id INTEGER NOT NULL REFERENCES images(id) ON DELETE CASCADE,
        tag_id   INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
        PRIMARY KEY (image_id, tag_id)
    ) WITHOUT ROWID;
    CREATE INDEX image_tags_by_tag ON image_tags (tag_id);",
    // 3: store image locations the same way on every system ("sub/image.jpg", not
    // "sub\image.jpg"), and stop the table itself from ignoring capital letters: whether
    // "Cat.jpg" and "cat.jpg" are the same file depends on the system, so lookups decide
    // that instead (see `same_path`). SQLite can't change a column like this in place, so
    // the table is rebuilt; ids are kept, so the links in image_tags still match.
    "CREATE TABLE images_new (
        id           INTEGER PRIMARY KEY,
        workspace_id INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
        rel_path     TEXT    NOT NULL,
        UNIQUE (workspace_id, rel_path)
    );
    INSERT INTO images_new (id, workspace_id, rel_path)
        SELECT id, workspace_id, replace(rel_path, '\\', '/') FROM images;
    DROP TABLE images;
    ALTER TABLE images_new RENAME TO images;
    CREATE INDEX images_by_path_any_case ON images (workspace_id, rel_path COLLATE NOCASE);",
];

/// SQL that compares a stored path with a parameter the way this system compares file names.
fn same_path(column: &str, param: &str) -> String {
    if PATHS_IGNORE_CASE {
        format!("{column} = {param} COLLATE NOCASE")
    } else {
        format!("{column} = {param}")
    }
}

/// Longest allowed tag name, in characters.
const MAX_TAG_LENGTH: usize = 60;

/// A named folder of images. Each workspace is browsed on its own.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub id: i64,
    pub name: String,
    pub path: String,
    /// Whether images in subfolders are included.
    pub recursive: bool,
}

#[derive(Serialize, Clone)]
pub struct Tag {
    pub id: i64,
    pub name: String,
}

/// Every tag in a workspace, and which tags each tagged image has.
#[derive(Serialize)]
pub struct TagData {
    pub tags: Vec<Tag>,
    pub images: Vec<ImageTags>,
}

#[derive(Serialize)]
pub struct ImageTags {
    /// Full path of the image, as the folder scan reports it.
    pub path: String,
    pub tags: Vec<i64>,
}

pub struct Library {
    conn: Mutex<Connection>,
}

impl Library {
    pub fn open(file: &Path) -> Result<Self> {
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir).map_err(err)?;
        }
        let mut conn = Connection::open(file).map_err(err)?;
        conn.pragma_update(None, "journal_mode", "WAL").map_err(err)?;
        // Links between tables are only enforced after upgrading: while a table is rebuilt,
        // enforcing them would delete everything linked to the old copy of it.
        conn.pragma_update(None, "foreign_keys", false).map_err(err)?;
        migrate(&mut conn)?;
        conn.pragma_update(None, "foreign_keys", true).map_err(err)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn workspaces(&self) -> Result<Vec<Workspace>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, name, path, recursive FROM workspaces ORDER BY position, id")
            .map_err(err)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Workspace { id: row.get(0)?, name: row.get(1)?, path: row.get(2)?, recursive: row.get(3)? })
            })
            .map_err(err)?;
        rows.collect::<std::result::Result<_, _>>().map_err(err)
    }

    /// Adds a folder as a new workspace, named after the folder. If a workspace
    /// for that folder already exists, that one is returned instead.
    pub fn add_workspace(&self, folder: &str) -> Result<Workspace> {
        let folder = folder_path(folder)?;
        let path = folder.to_string_lossy().into_owned();
        let name = folder
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());

        let id = {
            let conn = self.conn.lock().unwrap();
            let existing: Option<i64> = conn
                .query_row(
                    &format!("SELECT id FROM workspaces WHERE {}", same_path("path", "?1")),
                    [&path],
                    |r| r.get(0),
                )
                .optional()
                .map_err(err)?;
            match existing {
                Some(id) => id,
                None => {
                    conn.execute(
                        "INSERT INTO workspaces (name, path, position, created_at)
                         VALUES (?1, ?2, (SELECT COALESCE(MAX(position), 0) + 1 FROM workspaces), ?3)",
                        params![name, path, now()],
                    )
                    .map_err(err)?;
                    conn.last_insert_rowid()
                }
            }
        };
        self.workspace(id)
    }

    pub fn rename_workspace(&self, id: i64, name: &str) -> Result<Workspace> {
        let name = name.trim();
        if name.is_empty() {
            return Err("A workspace needs a name.".into());
        }
        self.update(id, "UPDATE workspaces SET name = ?2 WHERE id = ?1", name)
    }

    pub fn set_workspace_recursive(&self, id: i64, recursive: bool) -> Result<Workspace> {
        self.update(id, "UPDATE workspaces SET recursive = ?2 WHERE id = ?1", recursive)
    }

    /// Points an existing workspace at a different folder (e.g. after moving it).
    pub fn set_workspace_folder(&self, id: i64, folder: &str) -> Result<Workspace> {
        let path = folder_path(folder)?.to_string_lossy().into_owned();
        self.update(id, "UPDATE workspaces SET path = ?2 WHERE id = ?1", path)
    }

    /// Removes the workspace (and its tags) from the library. The folder and its images are not touched.
    pub fn remove_workspace(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM workspaces WHERE id = ?1", [id]).map_err(err)?;
        Ok(())
    }

    // ---- Tags ----

    pub fn tag_data(&self, workspace_id: i64) -> Result<TagData> {
        let conn = self.conn.lock().unwrap();
        tag_data(&conn, workspace_id)
    }

    /// Adds a tag to images, creating the tag if the workspace doesn't have it yet.
    /// Tag names aren't case-sensitive: adding "Anatomy" reuses an existing "anatomy".
    pub fn add_tag(&self, workspace_id: i64, paths: &[String], name: &str) -> Result<Tag> {
        let name = clean_tag_name(name)?;
        let mut conn = self.conn.lock().unwrap();
        let folder = PathBuf::from(get_workspace(&conn, workspace_id)?.path);
        let tx = conn.transaction().map_err(err)?;

        let existing: Option<Tag> = tx
            .query_row(
                "SELECT id, name FROM tags WHERE workspace_id = ?1 AND name = ?2",
                params![workspace_id, name],
                |r| Ok(Tag { id: r.get(0)?, name: r.get(1)? }),
            )
            .optional()
            .map_err(err)?;
        let tag = match existing {
            Some(tag) => tag,
            None => {
                tx.execute(
                    "INSERT INTO tags (workspace_id, name, created_at) VALUES (?1, ?2, ?3)",
                    params![workspace_id, name, now()],
                )
                .map_err(err)?;
                Tag { id: tx.last_insert_rowid(), name }
            }
        };

        {
            let mut find_image = tx
                .prepare(&format!(
                    "SELECT id FROM images WHERE workspace_id = ?1 AND {}",
                    same_path("rel_path", "?2")
                ))
                .map_err(err)?;
            let mut add_image =
                tx.prepare("INSERT INTO images (workspace_id, rel_path) VALUES (?1, ?2)").map_err(err)?;
            let mut link =
                tx.prepare("INSERT OR IGNORE INTO image_tags (image_id, tag_id) VALUES (?1, ?2)").map_err(err)?;
            for path in paths {
                let rel = relative_path(&folder, path)?;
                let found: Option<i64> =
                    find_image.query_row(params![workspace_id, rel], |r| r.get(0)).optional().map_err(err)?;
                let image_id = match found {
                    Some(id) => id,
                    None => {
                        add_image.execute(params![workspace_id, rel]).map_err(err)?;
                        tx.last_insert_rowid()
                    }
                };
                link.execute(params![image_id, tag.id]).map_err(err)?;
            }
        }
        tx.commit().map_err(err)?;
        Ok(tag)
    }

    /// Removes a tag from images. Returns true if no image has the tag any more,
    /// in which case the tag itself is deleted too.
    pub fn remove_tag(&self, workspace_id: i64, paths: &[String], tag_id: i64) -> Result<bool> {
        let mut conn = self.conn.lock().unwrap();
        let folder = PathBuf::from(get_workspace(&conn, workspace_id)?.path);
        let tx = conn.transaction().map_err(err)?;
        {
            let mut unlink = tx
                .prepare(&format!(
                    "DELETE FROM image_tags WHERE tag_id = ?1 AND image_id IN
                       (SELECT id FROM images WHERE workspace_id = ?2 AND {})",
                    same_path("rel_path", "?3")
                ))
                .map_err(err)?;
            for path in paths {
                unlink.execute(params![tag_id, workspace_id, relative_path(&folder, path)?]).map_err(err)?;
            }
        }
        let deleted = tx
            .execute(
                "DELETE FROM tags WHERE id = ?1 AND workspace_id = ?2
                   AND NOT EXISTS (SELECT 1 FROM image_tags WHERE tag_id = ?1)",
                params![tag_id, workspace_id],
            )
            .map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(deleted > 0)
    }

    /// Renames a tag. Renaming it to the name of another existing tag merges the two.
    pub fn rename_tag(&self, workspace_id: i64, tag_id: i64, name: &str) -> Result<TagData> {
        let name = clean_tag_name(name)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(err)?;
        let other: Option<i64> = tx
            .query_row(
                "SELECT id FROM tags WHERE workspace_id = ?1 AND name = ?2 AND id != ?3",
                params![workspace_id, name, tag_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        match other {
            Some(target) => {
                tx.execute(
                    "INSERT OR IGNORE INTO image_tags (image_id, tag_id)
                     SELECT image_id, ?1 FROM image_tags WHERE tag_id = ?2",
                    params![target, tag_id],
                )
                .map_err(err)?;
                tx.execute("DELETE FROM tags WHERE id = ?1 AND workspace_id = ?2", params![tag_id, workspace_id])
                    .map_err(err)?;
            }
            None => {
                tx.execute(
                    "UPDATE tags SET name = ?3 WHERE id = ?1 AND workspace_id = ?2",
                    params![tag_id, workspace_id, name],
                )
                .map_err(err)?;
            }
        }
        tx.commit().map_err(err)?;
        tag_data(&conn, workspace_id)
    }

    /// Deletes a tag and removes it from every image. The image files are not touched.
    pub fn delete_tag(&self, workspace_id: i64, tag_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM tags WHERE id = ?1 AND workspace_id = ?2", params![tag_id, workspace_id])
            .map_err(err)?;
        Ok(())
    }

    fn workspace(&self, id: i64) -> Result<Workspace> {
        let conn = self.conn.lock().unwrap();
        get_workspace(&conn, id)
    }

    fn update(&self, id: i64, sql: &str, value: impl rusqlite::ToSql) -> Result<Workspace> {
        {
            let conn = self.conn.lock().unwrap();
            conn.execute(sql, params![id, value]).map_err(err)?;
        }
        self.workspace(id)
    }
}

fn get_workspace(conn: &Connection, id: i64) -> Result<Workspace> {
    conn.query_row("SELECT id, name, path, recursive FROM workspaces WHERE id = ?1", [id], |row| {
        Ok(Workspace { id: row.get(0)?, name: row.get(1)?, path: row.get(2)?, recursive: row.get(3)? })
    })
    .map_err(|_| "That workspace no longer exists.".to_string())
}

fn tag_data(conn: &Connection, workspace_id: i64) -> Result<TagData> {
    let folder = PathBuf::from(get_workspace(conn, workspace_id)?.path);

    let mut stmt = conn
        .prepare("SELECT id, name FROM tags WHERE workspace_id = ?1 ORDER BY name")
        .map_err(err)?;
    let tags = stmt
        .query_map([workspace_id], |r| Ok(Tag { id: r.get(0)?, name: r.get(1)? }))
        .map_err(err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(err)?;

    let mut stmt = conn
        .prepare(
            "SELECT i.id, i.rel_path, it.tag_id FROM image_tags it
             JOIN images i ON i.id = it.image_id
             WHERE i.workspace_id = ?1 ORDER BY i.id",
        )
        .map_err(err)?;
    let mut rows = stmt.query([workspace_id]).map_err(err)?;
    let mut images: Vec<ImageTags> = Vec::new();
    let mut last_image = None;
    while let Some(row) = rows.next().map_err(err)? {
        let image_id: i64 = row.get(0).map_err(err)?;
        let tag_id: i64 = row.get(2).map_err(err)?;
        if last_image != Some(image_id) {
            let rel: String = row.get(1).map_err(err)?;
            images.push(ImageTags { path: full_path(&folder, &rel), tags: Vec::new() });
            last_image = Some(image_id);
        }
        images.last_mut().unwrap().tags.push(tag_id);
    }
    Ok(TagData { tags, images })
}

/// An image's location inside its workspace folder, which is how the database identifies it.
/// Always written with `/` between folders ("sub/image.jpg"), whatever the system uses.
fn relative_path(folder: &Path, path: &str) -> Result<String> {
    let rel = Path::new(path)
        .strip_prefix(folder)
        .map_err(|_| format!("{path} is not inside this workspace's folder."))?;
    let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    Ok(parts.join("/"))
}

/// The reverse of `relative_path`: the full path, written the way this system (and the folder scan) writes it.
fn full_path(folder: &Path, rel: &str) -> String {
    let mut path = folder.to_path_buf();
    for part in rel.split('/') {
        path.push(part);
    }
    path.to_string_lossy().into_owned()
}

/// Tidies up a tag name: trims it and squeezes repeated spaces.
fn clean_tag_name(name: &str) -> Result<String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err("A tag needs a name.".into());
    }
    if name.contains(',') {
        return Err("Tag names can't contain commas.".into());
    }
    if name.chars().count() > MAX_TAG_LENGTH {
        return Err(format!("Tag names can be at most {MAX_TAG_LENGTH} characters long."));
    }
    Ok(name)
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let applied: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0)).map_err(err)?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(applied as usize) {
        let tx = conn.transaction().map_err(err)?;
        tx.execute_batch(sql).map_err(err)?;
        // Make sure every link still points at something before saving the change.
        // If not, nothing is saved (the transaction is undone) and the app reports the problem.
        let broken_links: i64 = tx
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| r.get(0))
            .map_err(err)?;
        if broken_links > 0 {
            return Err(format!("Upgrading the library failed at step {} ({broken_links} broken links).", i + 1));
        }
        tx.pragma_update(None, "user_version", (i + 1) as i64).map_err(err)?;
        tx.commit().map_err(err)?;
    }
    Ok(())
}

/// The folder a workspace should use: the path itself, or its parent folder if it's a file.
fn folder_path(path: &str) -> Result<PathBuf> {
    let mut folder = PathBuf::from(path);
    if folder.is_file() {
        folder = folder.parent().map(PathBuf::from).unwrap_or(folder);
    }
    if !folder.is_dir() {
        return Err(format!("Folder not found: {path}"));
    }
    Ok(folder)
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh library in a temporary folder, with one workspace whose folder exists.
    fn setup(name: &str) -> (Library, Workspace, PathBuf) {
        let dir = std::env::temp_dir().join(format!("reflib-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let images = dir.join("Art");
        fs::create_dir_all(images.join("sub")).unwrap();
        let library = Library::open(&dir.join("library.db")).unwrap();
        let workspace = library.add_workspace(images.to_str().unwrap()).unwrap();
        (library, workspace, images)
    }

    /// A full path inside `folder`, written with this system's own separator
    /// ("sub/b.jpg" becomes "…\sub\b.jpg" on Windows and "…/sub/b.jpg" elsewhere).
    fn path(folder: &Path, rel: &str) -> String {
        full_path(folder, rel)
    }

    /// The image locations as stored in the database.
    fn stored_paths(lib: &Library) -> Vec<String> {
        let conn = lib.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT rel_path FROM images ORDER BY id").unwrap();
        stmt.query_map([], |r| r.get(0)).unwrap().map(|p| p.unwrap()).collect()
    }

    fn tags_of(data: &TagData, path: &str) -> Vec<String> {
        let mut names: Vec<String> = data
            .images
            .iter()
            .filter(|i| i.path == path)
            .flat_map(|i| i.tags.iter())
            .map(|id| data.tags.iter().find(|t| t.id == *id).unwrap().name.clone())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn add_tag_to_several_images_and_reuse_names_case_insensitively() {
        let (lib, ws, folder) = setup("add");
        let (a, b) = (path(&folder, "a.jpg"), path(&folder, "sub/b.jpg"));
        let tag = lib.add_tag(ws.id, &[a.clone(), b.clone()], "  Figure   drawing ").unwrap();
        assert_eq!(tag.name, "Figure drawing");
        let again = lib.add_tag(ws.id, &[a.clone()], "figure DRAWING").unwrap();
        assert_eq!(again.id, tag.id, "same tag regardless of case");
        assert_eq!(again.name, "Figure drawing", "keeps the original spelling");

        let data = lib.tag_data(ws.id).unwrap();
        assert_eq!(data.tags.len(), 1);
        assert_eq!(tags_of(&data, &a), ["Figure drawing"]);
        assert_eq!(tags_of(&data, &b), ["Figure drawing"]);
    }

    #[test]
    fn removing_a_tag_from_its_last_image_deletes_the_tag() {
        let (lib, ws, folder) = setup("remove");
        let (a, b) = (path(&folder, "a.jpg"), path(&folder, "b.jpg"));
        let tag = lib.add_tag(ws.id, &[a.clone(), b.clone()], "hands").unwrap();
        assert!(!lib.remove_tag(ws.id, &[a.clone()], tag.id).unwrap(), "still on b");
        assert_eq!(tags_of(&lib.tag_data(ws.id).unwrap(), &a), Vec::<String>::new());
        assert!(lib.remove_tag(ws.id, &[b.clone()], tag.id).unwrap(), "now unused");
        assert!(lib.tag_data(ws.id).unwrap().tags.is_empty());
    }

    #[test]
    fn renaming_onto_an_existing_tag_merges_them() {
        let (lib, ws, folder) = setup("merge");
        let (a, b) = (path(&folder, "a.jpg"), path(&folder, "b.jpg"));
        let hand = lib.add_tag(ws.id, &[a.clone(), b.clone()], "hand").unwrap();
        lib.add_tag(ws.id, &[b.clone()], "hands").unwrap();
        let data = lib.rename_tag(ws.id, hand.id, "Hands").unwrap();
        assert_eq!(data.tags.len(), 1, "merged into one tag");
        assert_eq!(tags_of(&data, &a), ["hands"]);
        assert_eq!(tags_of(&data, &b), ["hands"], "no duplicate on the image that had both");

        let plain = lib.rename_tag(ws.id, data.tags[0].id, "hand pose").unwrap();
        assert_eq!(tags_of(&plain, &a), ["hand pose"]);
    }

    #[test]
    fn tags_are_per_workspace_and_removed_with_it() {
        let (lib, art, folder) = setup("workspaces");
        let other_dir = folder.parent().unwrap().join("Home");
        fs::create_dir_all(&other_dir).unwrap();
        let home = lib.add_workspace(other_dir.to_str().unwrap()).unwrap();

        lib.add_tag(art.id, &[path(&folder, "a.jpg")], "blue").unwrap();
        let home_blue = lib.add_tag(home.id, &[path(&other_dir, "kitchen.jpg")], "blue").unwrap();
        assert_eq!(lib.tag_data(art.id).unwrap().tags.len(), 1);
        assert_eq!(lib.tag_data(home.id).unwrap().tags.len(), 1);
        assert_ne!(lib.tag_data(art.id).unwrap().tags[0].id, home_blue.id, "separate tags");

        lib.remove_workspace(art.id).unwrap();
        let conn = lib.conn.lock().unwrap();
        let leftover: i64 = conn
            .query_row("SELECT (SELECT COUNT(*) FROM tags) + (SELECT COUNT(*) FROM images)", [], |r| r.get(0))
            .unwrap();
        assert_eq!(leftover, 2, "only Home's tag and image remain");
    }

    #[test]
    fn paths_outside_the_workspace_are_refused() {
        let (lib, ws, _) = setup("outside");
        let outside = full_path(&std::env::temp_dir(), "elsewhere.jpg");
        assert!(lib.add_tag(ws.id, &[outside], "x").is_err());
        assert!(lib.add_tag(ws.id, &[], "bad, name").is_err());
    }

    #[test]
    fn tags_survive_moving_the_workspace_folder() {
        let (lib, ws, folder) = setup("move");
        lib.add_tag(ws.id, &[path(&folder, "sub/b.jpg")], "kept").unwrap();
        let moved = folder.parent().unwrap().join("Art (moved)");
        fs::rename(&folder, &moved).unwrap();
        lib.set_workspace_folder(ws.id, moved.to_str().unwrap()).unwrap();
        let data = lib.tag_data(ws.id).unwrap();
        assert_eq!(tags_of(&data, &path(&moved, "sub/b.jpg")), ["kept"]);
    }

    #[test]
    fn locations_are_stored_the_same_way_on_every_system() {
        let (lib, ws, folder) = setup("portable");
        lib.add_tag(ws.id, &[path(&folder, "sub/b.jpg")], "x").unwrap();
        assert_eq!(stored_paths(&lib), ["sub/b.jpg"]);
    }

    #[test]
    fn upgrading_an_old_library_keeps_its_tags() {
        // Build a library as version 0.3.0 left it: Windows-style locations, case-insensitive table.
        let dir = std::env::temp_dir().join(format!("reflib-test-upgrade-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let folder = dir.join("Art");
        fs::create_dir_all(&folder).unwrap();
        let file = dir.join("library.db");
        {
            let conn = Connection::open(&file).unwrap();
            for sql in &MIGRATIONS[..2] {
                conn.execute_batch(sql).unwrap();
            }
            conn.pragma_update(None, "user_version", 2).unwrap();
            conn.execute(
                "INSERT INTO workspaces (id, name, path, position, created_at) VALUES (1, 'Art', ?1, 1, 0)",
                [folder.to_str().unwrap()],
            )
            .unwrap();
            conn.execute_batch(
                r"INSERT INTO images (id, workspace_id, rel_path) VALUES (10, 1, 'a.jpg'), (11, 1, 'sub\deeper\b.jpg');
                  INSERT INTO tags (id, workspace_id, name, created_at) VALUES (5, 1, 'figure', 0), (6, 1, 'pose', 0);
                  INSERT INTO image_tags (image_id, tag_id) VALUES (10, 5), (11, 5), (11, 6);",
            )
            .unwrap();
        }

        let lib = Library::open(&file).unwrap();
        assert_eq!(stored_paths(&lib), ["a.jpg", "sub/deeper/b.jpg"], "separators converted");
        let data = lib.tag_data(1).unwrap();
        assert_eq!(tags_of(&data, &path(&folder, "a.jpg")), ["figure"]);
        assert_eq!(tags_of(&data, &path(&folder, "sub/deeper/b.jpg")), ["figure", "pose"], "links kept");

        // Removing the workspace still cleans everything up after the table was rebuilt.
        lib.remove_workspace(1).unwrap();
        let conn = lib.conn.lock().unwrap();
        let left: i64 = conn
            .query_row("SELECT (SELECT COUNT(*) FROM images) + (SELECT COUNT(*) FROM image_tags)", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn capital_letters_follow_the_system() {
        let (lib, ws, folder) = setup("case");
        lib.add_tag(ws.id, &[path(&folder, "sub/Cat.jpg")], "cat").unwrap();
        lib.add_tag(ws.id, &[path(&folder, "sub/cat.jpg")], "cat").unwrap();
        let images = stored_paths(&lib).len();
        if PATHS_IGNORE_CASE {
            assert_eq!(images, 1, "Windows and macOS: the same file");
        } else {
            assert_eq!(images, 2, "Linux: two different files");
        }
    }
}
