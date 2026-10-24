//! The library database: the list of workspaces, and the tags on their images.
//!
//! It is a single SQLite file in the app's data folder
//! (`%APPDATA%\com.kaigulliksen.reflibrary\library.db` on Windows).
//! Only the library's own information lives here; image files are never
//! moved, copied or changed.

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
];

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
        conn.pragma_update(None, "foreign_keys", true).map_err(err)?;
        migrate(&mut conn)?;
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
            // Windows paths aren't case-sensitive, so neither is this check.
            let existing: Option<i64> = conn
                .query_row("SELECT id FROM workspaces WHERE lower(path) = lower(?1)", [&path], |r| r.get(0))
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
            let mut add_image = tx
                .prepare("INSERT INTO images (workspace_id, rel_path) VALUES (?1, ?2) ON CONFLICT DO NOTHING")
                .map_err(err)?;
            let mut find_image =
                tx.prepare("SELECT id FROM images WHERE workspace_id = ?1 AND rel_path = ?2").map_err(err)?;
            let mut link =
                tx.prepare("INSERT OR IGNORE INTO image_tags (image_id, tag_id) VALUES (?1, ?2)").map_err(err)?;
            for path in paths {
                let rel = relative_path(&folder, path)?;
                add_image.execute(params![workspace_id, rel]).map_err(err)?;
                let image_id: i64 =
                    find_image.query_row(params![workspace_id, rel], |r| r.get(0)).map_err(err)?;
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
                .prepare(
                    "DELETE FROM image_tags WHERE tag_id = ?1 AND image_id =
                       (SELECT id FROM images WHERE workspace_id = ?2 AND rel_path = ?3)",
                )
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
            images.push(ImageTags { path: folder.join(rel).to_string_lossy().into_owned(), tags: Vec::new() });
            last_image = Some(image_id);
        }
        images.last_mut().unwrap().tags.push(tag_id);
    }
    Ok(TagData { tags, images })
}

/// An image's path relative to its workspace folder, which is how the database identifies it.
fn relative_path(folder: &Path, path: &str) -> Result<String> {
    Path::new(path)
        .strip_prefix(folder)
        .map(|rel| rel.to_string_lossy().into_owned())
        .map_err(|_| format!("{path} is not inside this workspace's folder."))
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

    fn path(folder: &Path, rel: &str) -> String {
        folder.join(rel).to_string_lossy().into_owned()
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
        let (a, b) = (path(&folder, "a.jpg"), path(&folder, r"sub\b.jpg"));
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
        assert!(lib.add_tag(ws.id, &[r"C:\Windows\notepad.exe".into()], "x").is_err());
        assert!(lib.add_tag(ws.id, &[], "bad, name").is_err());
    }

    #[test]
    fn tags_survive_moving_the_workspace_folder() {
        let (lib, ws, folder) = setup("move");
        lib.add_tag(ws.id, &[path(&folder, r"sub\b.jpg")], "kept").unwrap();
        let moved = folder.parent().unwrap().join("Art (moved)");
        fs::rename(&folder, &moved).unwrap();
        lib.set_workspace_folder(ws.id, moved.to_str().unwrap()).unwrap();
        let data = lib.tag_data(ws.id).unwrap();
        assert_eq!(tags_of(&data, &path(&moved, r"sub\b.jpg")), ["kept"]);
    }
}
