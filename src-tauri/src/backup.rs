//! Safety copies of the library database, in a `backups` folder next to it.
//!
//! All tags and boards live in that one file, so it gets copied in two situations:
//! - once a day, when the app starts (the last `KEEP_DAILY` days are kept);
//! - right before a new version upgrades the database. These are kept for good: there's only one
//!   per upgrade, and they're the way back if an upgrade ever goes wrong.
//!
//! `VACUUM INTO` is SQLite's own way of making a copy. Unlike copying the file, it gives a complete,
//! consistent database even while the app is using the original. Copies are written under a
//! temporary name first, so a half-written one can't be mistaken for a real backup.

use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// How many daily copies to keep.
const KEEP_DAILY: usize = 7;
const DAILY_PREFIX: &str = "library-";
const SECONDS_PER_DAY: u64 = 24 * 60 * 60;

/// The folder the backups of this database go in.
pub fn folder(database: &Path) -> PathBuf {
    database.parent().unwrap_or(Path::new(".")).join("backups")
}

/// Copies the database before it's upgraded from schema step `version`. If that copy already
/// exists (an earlier upgrade attempt failed), it's kept: it's the one from before any attempt.
pub fn before_upgrade(conn: &Connection, database: &Path, version: i64) -> Result<(), String> {
    let copy = folder(database).join(format!("library-before-upgrade-from-step-{version}.db"));
    if copy.exists() {
        return Ok(());
    }
    write_copy(conn, &copy)
}

/// Makes today's copy of the database, if it hasn't been made yet, and clears out old ones.
/// Opens its own connection, so it can run in the background without holding up the app.
pub fn daily(database: &Path) -> Result<(), String> {
    let today = date_name(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) / SECONDS_PER_DAY);
    daily_on(database, &today)
}

fn daily_on(database: &Path, today: &str) -> Result<(), String> {
    let copy = folder(database).join(format!("{DAILY_PREFIX}{today}.db"));
    if !copy.exists() {
        let conn = Connection::open(database).map_err(|e| e.to_string())?;
        write_copy(&conn, &copy)?;
    }
    remove_old_copies(&folder(database));
    Ok(())
}

fn write_copy(conn: &Connection, copy: &Path) -> Result<(), String> {
    let dir = copy.parent().ok_or("A backup needs a folder to go in.")?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let temporary = copy.with_extension("tmp");
    let _ = fs::remove_file(&temporary); // left over from a copy that was interrupted
    let made = conn
        .execute("VACUUM INTO ?1", [temporary.to_string_lossy().as_ref()])
        .map_err(|e| e.to_string())
        .and_then(|_| fs::rename(&temporary, copy).map_err(|e| e.to_string()));
    if made.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    made
}

/// Keeps the newest daily copies. Their names hold the date, so sorting by name sorts by age.
/// Copies from before upgrades have a different name and are left alone.
fn remove_old_copies(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut daily: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| is_daily_copy(p)).collect();
    daily.sort();
    daily.reverse();
    for old in daily.iter().skip(KEEP_DAILY) {
        let _ = fs::remove_file(old);
    }
}

/// Whether a file is named like a daily copy: "library-2026-11-01.db".
fn is_daily_copy(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "db")
        && path.file_stem().and_then(|stem| stem.to_str()).and_then(|stem| stem.strip_prefix(DAILY_PREFIX)).is_some_and(
            |date| date.len() == 10 && date.chars().enumerate().all(|(i, c)| if i == 4 || i == 7 { c == '-' } else { c.is_ascii_digit() }),
        )
}

/// A date as "YYYY-MM-DD", from the number of days since 1970 (in UTC, which is close enough for
/// "about once a day"). A few lines of arithmetic instead of a whole date library.
fn date_name(days: u64) -> String {
    // Howard Hinnant's "civil from days": works in 400-year cycles of years starting in March.
    let z = days + 719_468;
    let era = z / 146_097;
    let day_of_era = z % 146_097;
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 { shifted_month + 3 } else { shifted_month - 9 };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("reflib-test-backup-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("library.db");
        let conn = Connection::open(&file).unwrap();
        conn.execute_batch("CREATE TABLE t (x); INSERT INTO t VALUES (42);").unwrap();
        file
    }

    fn value_in(file: &Path) -> i64 {
        Connection::open(file).unwrap().query_row("SELECT x FROM t", [], |r| r.get(0)).unwrap()
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> =
            fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn dates_are_written_correctly() {
        assert_eq!(date_name(0), "1970-01-01");
        assert_eq!(date_name(59), "1970-03-01");
        assert_eq!(date_name(11_016), "2000-02-29"); // a leap day
        assert_eq!(date_name(20_758), "2026-11-01");
    }

    #[test]
    fn a_daily_copy_is_made_once_and_holds_the_data() {
        let file = database("once");
        daily_on(&file, "2026-11-01").unwrap();
        let copy = folder(&file).join("library-2026-11-01.db");
        assert_eq!(value_in(&copy), 42);

        // A second start on the same day leaves the first copy as it was.
        Connection::open(&file).unwrap().execute("UPDATE t SET x = 7", []).unwrap();
        daily_on(&file, "2026-11-01").unwrap();
        assert_eq!(value_in(&copy), 42);
        daily_on(&file, "2026-11-02").unwrap();
        assert_eq!(value_in(&folder(&file).join("library-2026-11-02.db")), 7);
    }

    #[test]
    fn only_the_newest_daily_copies_are_kept_and_nothing_else_is_removed() {
        let file = database("prune");
        let conn = Connection::open(&file).unwrap();
        before_upgrade(&conn, &file, 5).unwrap();
        let mine = folder(&file).join("my own copy.db");
        fs::write(&mine, "kept").unwrap();
        for day in 1..=9 {
            daily_on(&file, &format!("2026-11-{day:02}")).unwrap();
        }
        assert_eq!(
            names(&folder(&file)),
            [
                "library-2026-11-03.db",
                "library-2026-11-04.db",
                "library-2026-11-05.db",
                "library-2026-11-06.db",
                "library-2026-11-07.db",
                "library-2026-11-08.db",
                "library-2026-11-09.db",
                "library-before-upgrade-from-step-5.db",
                "my own copy.db",
            ]
        );
    }

    #[test]
    fn a_copy_from_before_an_upgrade_is_never_replaced() {
        let file = database("upgrade");
        let conn = Connection::open(&file).unwrap();
        before_upgrade(&conn, &file, 3).unwrap();
        conn.execute("UPDATE t SET x = 7", []).unwrap();
        before_upgrade(&conn, &file, 3).unwrap();
        assert_eq!(value_in(&folder(&file).join("library-before-upgrade-from-step-3.db")), 42);
    }
}
