//! Mood boards: free-form canvases of images (and text notes) from one workspace.
//!
//! Board images point at the library's image records (see `image_record`), not at copies,
//! so they take no extra disk space and follow renamed or moved files the same way tags do.

use crate::library::{
    Library, Result, err, full_path, get_workspace, image_record, now, relative_path,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Board {
    pub id: i64,
    pub workspace_id: i64,
    pub name: String,
    /// Number of images on the board (notes not counted).
    pub image_count: i64,
}

/// Where the board was last looked at. A zoom of 0 means it hasn't been opened yet.
#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct BoardView {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

/// The part of an image shown on the board, as fractions (0–1) of its width and height.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
pub struct Crop {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// An image or a note on a board. Exactly one of `path` and `text` is set.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BoardItem {
    pub id: i64,
    /// Full path of the image (as the folder scan writes it), for images.
    pub path: Option<String>,
    /// The text, for notes.
    pub text: Option<String>,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Stacking order: higher is in front.
    pub z: i64,
    pub crop: Option<Crop>,
}

#[derive(Serialize)]
pub struct BoardData {
    pub board: Board,
    pub view: BoardView,
    pub items: Vec<BoardItem>,
}

/// Where to put an image that's being added.
#[derive(Deserialize)]
pub struct NewImage {
    pub path: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Library {
    pub fn boards(&self, workspace_id: i64) -> Result<Vec<Board>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT b.id, b.workspace_id, b.name,
                        (SELECT COUNT(*) FROM board_items i WHERE i.board_id = b.id AND i.image_id IS NOT NULL)
                 FROM boards b WHERE b.workspace_id = ?1 ORDER BY b.position, b.id",
            )
            .map_err(err)?;
        stmt.query_map([workspace_id], |r| {
            Ok(Board { id: r.get(0)?, workspace_id: r.get(1)?, name: r.get(2)?, image_count: r.get(3)? })
        })
        .map_err(err)?
        .collect::<std::result::Result<_, _>>()
        .map_err(err)
    }

    pub fn create_board(&self, workspace_id: i64, name: &str) -> Result<Board> {
        let name = board_name(name)?;
        let conn = self.conn.lock().unwrap();
        get_workspace(&conn, workspace_id)?;
        conn.execute(
            "INSERT INTO boards (workspace_id, name, position, created_at)
             VALUES (?1, ?2, (SELECT COALESCE(MAX(position), 0) + 1 FROM boards WHERE workspace_id = ?1), ?3)",
            params![workspace_id, name, now()],
        )
        .map_err(err)?;
        get_board(&conn, conn.last_insert_rowid())
    }

    pub fn rename_board(&self, id: i64, name: &str) -> Result<Board> {
        let name = board_name(name)?;
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE boards SET name = ?2 WHERE id = ?1", params![id, name]).map_err(err)?;
        get_board(&conn, id)
    }

    /// Deletes a board and everything on it. The images themselves are not touched.
    pub fn delete_board(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM boards WHERE id = ?1", [id]).map_err(err)?;
        Ok(())
    }

    pub fn board_data(&self, id: i64) -> Result<BoardData> {
        let conn = self.conn.lock().unwrap();
        let board = get_board(&conn, id)?;
        let folder = PathBuf::from(get_workspace(&conn, board.workspace_id)?.path);
        let view = conn
            .query_row("SELECT view_x, view_y, view_zoom FROM boards WHERE id = ?1", [id], |r| {
                Ok(BoardView { x: r.get(0)?, y: r.get(1)?, zoom: r.get(2)? })
            })
            .map_err(err)?;
        let mut stmt = conn
            .prepare(
                "SELECT bi.id, img.rel_path, bi.text, bi.x, bi.y, bi.width, bi.height, bi.z,
                        bi.crop_x, bi.crop_y, bi.crop_w, bi.crop_h
                 FROM board_items bi LEFT JOIN images img ON img.id = bi.image_id
                 WHERE bi.board_id = ?1 ORDER BY bi.z, bi.id",
            )
            .map_err(err)?;
        let items = stmt
            .query_map([id], |r| {
                let rel: Option<String> = r.get(1)?;
                let crop = match (r.get(8)?, r.get(9)?, r.get(10)?, r.get(11)?) {
                    (Some(x), Some(y), Some(w), Some(h)) => Some(Crop { x, y, w, h }),
                    _ => None,
                };
                Ok(BoardItem {
                    id: r.get(0)?,
                    path: rel.map(|rel| full_path(&folder, &rel)),
                    text: r.get(2)?,
                    x: r.get(3)?,
                    y: r.get(4)?,
                    width: r.get(5)?,
                    height: r.get(6)?,
                    z: r.get(7)?,
                    crop,
                })
            })
            .map_err(err)?
            .collect::<std::result::Result<_, _>>()
            .map_err(err)?;
        Ok(BoardData { board, view, items })
    }

    /// Puts images on a board, in front of everything already there. Images that are already
    /// on the board are skipped. Returns the new items.
    pub fn add_board_images(&self, board_id: i64, images: &[NewImage]) -> Result<Vec<BoardItem>> {
        let mut conn = self.conn.lock().unwrap();
        let board = get_board(&conn, board_id)?;
        let folder = PathBuf::from(get_workspace(&conn, board.workspace_id)?.path);
        let tx = conn.transaction().map_err(err)?;
        let mut z = top_z(&tx, board_id)?;
        let mut added = Vec::new();
        for image in images {
            let image_id = image_record(&tx, board.workspace_id, &relative_path(&folder, &image.path)?)?;
            let already: bool = tx
                .query_row(
                    "SELECT EXISTS (SELECT 1 FROM board_items WHERE board_id = ?1 AND image_id = ?2)",
                    params![board_id, image_id],
                    |r| r.get(0),
                )
                .map_err(err)?;
            if already {
                continue;
            }
            z += 1;
            tx.execute(
                "INSERT INTO board_items (board_id, image_id, x, y, width, height, z) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![board_id, image_id, image.x, image.y, image.width, image.height, z],
            )
            .map_err(err)?;
            added.push(BoardItem {
                id: tx.last_insert_rowid(),
                path: Some(image.path.clone()),
                text: None,
                x: image.x,
                y: image.y,
                width: image.width,
                height: image.height,
                z,
                crop: None,
            });
        }
        tx.commit().map_err(err)?;
        Ok(added)
    }

    pub fn add_board_note(&self, board_id: i64, text: &str, x: f64, y: f64, width: f64, height: f64) -> Result<BoardItem> {
        let conn = self.conn.lock().unwrap();
        get_board(&conn, board_id)?;
        let z = top_z(&conn, board_id)? + 1;
        conn.execute(
            "INSERT INTO board_items (board_id, text, x, y, width, height, z) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![board_id, text, x, y, width, height, z],
        )
        .map_err(err)?;
        Ok(BoardItem { id: conn.last_insert_rowid(), path: None, text: Some(text.into()), x, y, width, height, z, crop: None })
    }

    /// Saves the position, size, stacking order, crop and (for notes) text of items on a board.
    pub fn update_board_items(&self, board_id: i64, items: &[BoardItem]) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction().map_err(err)?;
        {
            let mut update = tx
                .prepare(
                    "UPDATE board_items SET x = ?3, y = ?4, width = ?5, height = ?6, z = ?7,
                         crop_x = ?8, crop_y = ?9, crop_w = ?10, crop_h = ?11,
                         text = CASE WHEN image_id IS NULL THEN ?12 ELSE NULL END
                     WHERE id = ?1 AND board_id = ?2",
                )
                .map_err(err)?;
            for item in items {
                let crop = item.crop;
                update
                    .execute(params![
                        item.id,
                        board_id,
                        item.x,
                        item.y,
                        item.width,
                        item.height,
                        item.z,
                        crop.map(|c| c.x),
                        crop.map(|c| c.y),
                        crop.map(|c| c.w),
                        crop.map(|c| c.h),
                        item.text.clone().unwrap_or_default(),
                    ])
                    .map_err(err)?;
            }
        }
        tx.commit().map_err(err)
    }

    /// Takes items off a board. Images stay in the library; only their place on the board goes.
    pub fn remove_board_items(&self, board_id: i64, ids: &[i64]) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        for id in ids {
            conn.execute("DELETE FROM board_items WHERE id = ?1 AND board_id = ?2", params![id, board_id])
                .map_err(err)?;
        }
        Ok(())
    }

    pub fn set_board_view(&self, board_id: i64, view: BoardView) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE boards SET view_x = ?2, view_y = ?3, view_zoom = ?4 WHERE id = ?1",
            params![board_id, view.x, view.y, view.zoom],
        )
        .map_err(err)?;
        Ok(())
    }
}

fn get_board(conn: &Connection, id: i64) -> Result<Board> {
    conn.query_row(
        "SELECT b.id, b.workspace_id, b.name,
                (SELECT COUNT(*) FROM board_items i WHERE i.board_id = b.id AND i.image_id IS NOT NULL)
         FROM boards b WHERE b.id = ?1",
        [id],
        |r| Ok(Board { id: r.get(0)?, workspace_id: r.get(1)?, name: r.get(2)?, image_count: r.get(3)? }),
    )
    .optional()
    .map_err(err)?
    .ok_or_else(|| "That board no longer exists.".to_string())
}

fn top_z(conn: &Connection, board_id: i64) -> Result<i64> {
    conn.query_row("SELECT COALESCE(MAX(z), 0) FROM board_items WHERE board_id = ?1", [board_id], |r| r.get(0))
        .map_err(err)
}

fn board_name(name: &str) -> Result<String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err("A board needs a name.".into());
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn setup(name: &str) -> (Library, i64, PathBuf) {
        let dir = std::env::temp_dir().join(format!("reflib-test-boards-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let folder = dir.join("Art");
        fs::create_dir_all(&folder).unwrap();
        let library = Library::open(&dir.join("library.db")).unwrap();
        let workspace = library.add_workspace(folder.to_str().unwrap()).unwrap();
        (library, workspace.id, folder)
    }

    fn write_image(folder: &Path, name: &str, seed: u8) -> String {
        let bytes: Vec<u8> = (0..40_000u32).map(|i| (i as u8).wrapping_mul(17).wrapping_add(seed)).collect();
        let path = folder.join(name);
        fs::write(&path, bytes).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn at(path: &str, x: f64) -> NewImage {
        NewImage { path: path.into(), x, y: 0.0, width: 300.0, height: 200.0 }
    }

    #[test]
    fn boards_hold_images_and_notes_and_save_changes() {
        let (lib, ws, folder) = setup("basic");
        let (a, b) = (write_image(&folder, "a.jpg", 1), write_image(&folder, "b.jpg", 2));
        let board = lib.create_board(ws, "  Forest   knight ").unwrap();
        assert_eq!(board.name, "Forest knight");

        let added = lib.add_board_images(board.id, &[at(&a, 0.0), at(&b, 320.0)]).unwrap();
        assert_eq!(added.len(), 2);
        assert!(added[1].z > added[0].z, "later images go in front");
        let again = lib.add_board_images(board.id, &[at(&a, 999.0)]).unwrap();
        assert!(again.is_empty(), "an image already on the board isn't added twice");

        let note = lib.add_board_note(board.id, "use this lighting", 10.0, 250.0, 200.0, 100.0).unwrap();
        assert!(note.z > added[1].z);

        let mut moved = added[0].clone();
        moved.x = 50.0;
        moved.crop = Some(Crop { x: 0.1, y: 0.2, w: 0.5, h: 0.5 });
        let mut edited = note.clone();
        edited.text = Some("cooler light".into());
        lib.update_board_items(board.id, &[moved, edited]).unwrap();

        let data = lib.board_data(board.id).unwrap();
        assert_eq!(data.board.image_count, 2);
        assert_eq!(data.view.zoom, 0.0, "never viewed yet");
        let first = data.items.iter().find(|i| i.id == added[0].id).unwrap();
        assert_eq!(first.x, 50.0);
        assert_eq!(first.crop, Some(Crop { x: 0.1, y: 0.2, w: 0.5, h: 0.5 }));
        assert_eq!(first.path.as_deref(), Some(a.as_str()));
        let saved_note = data.items.iter().find(|i| i.id == note.id).unwrap();
        assert_eq!(saved_note.text.as_deref(), Some("cooler light"));
        assert_eq!(saved_note.path, None);

        lib.remove_board_items(board.id, &[added[1].id]).unwrap();
        assert_eq!(lib.board_data(board.id).unwrap().board.image_count, 1);

        lib.set_board_view(board.id, BoardView { x: -40.0, y: 12.0, zoom: 0.75 }).unwrap();
        let view = lib.board_data(board.id).unwrap().view;
        assert_eq!((view.x, view.y, view.zoom), (-40.0, 12.0, 0.75));
    }

    #[test]
    fn board_images_follow_renamed_files_and_are_kept_without_tags() {
        let (lib, ws, folder) = setup("rename");
        let a = write_image(&folder, "a.jpg", 3);
        let board = lib.create_board(ws, "Board").unwrap();
        lib.add_board_images(board.id, &[at(&a, 0.0)]).unwrap();
        lib.fingerprint_new(ws, &[a.clone()]).unwrap();

        // Untagged, but on a board: the scan's clean-up must keep it.
        lib.reconcile(ws, &crate::scan::scan(&folder, false)).unwrap();
        assert_eq!(lib.board_data(board.id).unwrap().items.len(), 1);

        fs::rename(&a, folder.join("renamed.jpg")).unwrap();
        assert_eq!(lib.reconcile(ws, &crate::scan::scan(&folder, false)).unwrap(), 1);
        let item = &lib.board_data(board.id).unwrap().items[0];
        assert_eq!(item.path.as_deref(), Some(folder.join("renamed.jpg").to_str().unwrap()));
    }

    #[test]
    fn deleting_a_board_or_its_workspace_cleans_up() {
        let (lib, ws, folder) = setup("delete");
        let a = write_image(&folder, "a.jpg", 4);
        let keep = lib.create_board(ws, "Keep").unwrap();
        let gone = lib.create_board(ws, "Gone").unwrap();
        lib.add_board_images(keep.id, &[at(&a, 0.0)]).unwrap();
        lib.add_board_images(gone.id, &[at(&a, 0.0)]).unwrap();

        lib.delete_board(gone.id).unwrap();
        assert_eq!(lib.boards(ws).unwrap().iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["Keep"]);
        assert!(lib.board_data(gone.id).is_err());
        assert_eq!(lib.board_data(keep.id).unwrap().items.len(), 1, "the image is still on the other board");

        lib.remove_workspace(ws).unwrap();
        let conn = lib.conn.lock().unwrap();
        let left: i64 = conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM boards) + (SELECT COUNT(*) FROM board_items) + (SELECT COUNT(*) FROM images)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(left, 0);
        assert!(Path::new(&a).exists(), "the image file itself is never touched");
    }

    #[test]
    fn boards_belong_to_their_workspace() {
        let (lib, art, folder) = setup("separate");
        let home_dir = folder.parent().unwrap().join("Home");
        fs::create_dir_all(&home_dir).unwrap();
        let home = lib.add_workspace(home_dir.to_str().unwrap()).unwrap().id;
        let board = lib.create_board(art, "Art board").unwrap();
        lib.create_board(home, "Home board").unwrap();
        assert_eq!(lib.boards(art).unwrap().len(), 1);

        // Images from another workspace's folder can't go on this board.
        let outside = write_image(&home_dir, "kitchen.jpg", 5);
        assert!(lib.add_board_images(board.id, &[at(&outside, 0.0)]).is_err());
        assert!(lib.create_board(art, "   ").is_err());
    }
}
