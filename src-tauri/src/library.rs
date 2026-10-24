//! The library database: the list of workspaces (and, later, tags).
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
];

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

    /// Removes the workspace from the library. The folder and its images are not touched.
    pub fn remove_workspace(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM workspaces WHERE id = ?1", [id]).map_err(err)?;
        Ok(())
    }

    fn workspace(&self, id: i64) -> Result<Workspace> {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT id, name, path, recursive FROM workspaces WHERE id = ?1", [id], |row| {
            Ok(Workspace { id: row.get(0)?, name: row.get(1)?, path: row.get(2)?, recursive: row.get(3)? })
        })
        .map_err(|_| "That workspace no longer exists.".to_string())
    }

    fn update(&self, id: i64, sql: &str, value: impl rusqlite::ToSql) -> Result<Workspace> {
        {
            let conn = self.conn.lock().unwrap();
            conn.execute(sql, params![id, value]).map_err(err)?;
        }
        self.workspace(id)
    }
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
