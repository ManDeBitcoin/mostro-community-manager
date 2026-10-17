//! Read-only access to the daemon's database, `active/mostro.db`: a
//! consistent copy for backups, an integrity check, and whether trades are in
//! flight. mostrod owns the file and runs in another container; nothing here
//! writes to it, and every read goes through SQLite's own locks.
use rusqlite::{
    Connection, OpenFlags,
    backup::{Backup, StepResult},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// Order statuses with nothing left to settle (mostro-core `Status`). Any
/// other one, including a status a newer mostrod adds, counts as open.
const FINAL_STATUSES: &[&str] = &[
    "canceled",
    "canceled-by-admin",
    "settled-by-admin",
    "completed-by-admin",
    "expired",
    "success",
    "cooperatively-canceled",
];

/// The order is published and waiting to be taken: no funds are locked yet
/// unless a maker bond was paid.
const PENDING: &str = "pending";

/// How long a copy waits while mostrod holds the database locked.
const BUSY_LIMIT: Duration = Duration::from_secs(60);

pub fn database_path(root: &Path) -> PathBuf {
    root.join("active").join("mostro.db")
}

fn open_read_only(path: &Path) -> Result<Connection, &'static str> {
    if !path.is_file() {
        return Err("No hay base de datos de mostrod");
    }
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| "No se pudo abrir la base de mostrod")?;
    db.busy_timeout(Duration::from_secs(5))
        .map_err(|_| "No se pudo abrir la base de mostrod")?;
    Ok(db)
}

/// Copies the database into `destination`, a file that must not exist yet,
/// with SQLite's backup API: pages are read under SQLite's locks, so the copy
/// is consistent while mostrod keeps running.
pub fn snapshot(source: &Path, destination: &Path) -> Result<(), &'static str> {
    if destination.exists() {
        return Err("El destino de la copia ya existe");
    }
    let source = open_read_only(source)?;
    let mut copy =
        Connection::open(destination).map_err(|_| "No se pudo crear la copia de la base")?;
    let backup =
        Backup::new(&source, &mut copy).map_err(|_| "No se pudo iniciar la copia de la base")?;
    let deadline = Instant::now() + BUSY_LIMIT;
    loop {
        match backup
            .step(256)
            .map_err(|_| "La copia de la base de mostrod falló")?
        {
            StepResult::Done => return Ok(()),
            StepResult::More => {}
            _ if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            _ => return Err("La base de mostrod estuvo ocupada demasiado tiempo"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DbCheck {
    /// `ok`, or SQLite's first complaint.
    pub integrity: String,
    pub migrations: u64,
    pub failed_migrations: u64,
    pub latest_migration: Option<i64>,
}

impl DbCheck {
    pub fn healthy(&self) -> bool {
        self.integrity == "ok" && self.failed_migrations == 0
    }
}

/// SQLite counts are signed; they are never negative.
fn count(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

pub fn check(path: &Path) -> Result<DbCheck, &'static str> {
    let db = open_read_only(path)?;
    let integrity: String = db
        .query_row("pragma integrity_check", [], |row| row.get(0))
        .map_err(|_| "No se pudo comprobar la integridad de la base")?;
    let (migrations, failed_migrations, latest_migration): (i64, i64, Option<i64>) = db
        .query_row(
            "select count(*), count(*) filter (where success = 0), max(version) from _sqlx_migrations",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| "La base no tiene el registro de migraciones de mostrod")?;
    Ok(DbCheck {
        integrity,
        migrations: count(migrations),
        failed_migrations: count(failed_migrations),
        latest_migration,
    })
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OpenOrders {
    /// Taken or otherwise under way: an update must wait for them.
    pub in_flight: u64,
    /// Published offers nobody took yet.
    pub pending: u64,
    /// Every status that is not final, with its count.
    pub by_status: BTreeMap<String, u64>,
}

pub fn open_orders(path: &Path) -> Result<OpenOrders, &'static str> {
    let db = open_read_only(path)?;
    let mut statement = db
        .prepare("select status, count(*) from orders group by status")
        .map_err(|_| "No se pudieron leer las órdenes de mostrod")?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, count(row.get::<_, i64>(1)?)))
        })
        .map_err(|_| "No se pudieron leer las órdenes de mostrod")?;
    let mut open = OpenOrders::default();
    for row in rows {
        let (status, count) = row.map_err(|_| "No se pudieron leer las órdenes de mostrod")?;
        if FINAL_STATUSES.contains(&status.as_str()) {
            continue;
        }
        if status == PENDING {
            open.pending += count;
        } else {
            open.in_flight += count;
        }
        open.by_status.insert(status, count);
    }
    Ok(open)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database(dir: &Path, statuses: &[&str]) -> PathBuf {
        let path = dir.join("mostro.db");
        let db = Connection::open(&path).unwrap();
        db.execute_batch(
            "create table _sqlx_migrations (version integer primary key, success boolean not null);
             insert into _sqlx_migrations values (1, 1), (2, 1);
             create table orders (id text primary key, status text not null);",
        )
        .unwrap();
        for (n, status) in statuses.iter().enumerate() {
            db.execute(
                "insert into orders values (?1, ?2)",
                (n.to_string(), status),
            )
            .unwrap();
        }
        path
    }

    #[test]
    fn final_orders_are_not_open_and_unknown_ones_are() {
        let dir = tempfile::tempdir().unwrap();
        let path = database(
            dir.path(),
            &[
                "success",
                "canceled",
                "expired",
                "pending",
                "pending",
                "active",
                "fiat-sent",
                "a-future-status",
            ],
        );
        let open = open_orders(&path).unwrap();
        assert_eq!(open.pending, 2);
        assert_eq!(open.in_flight, 3);
        assert_eq!(open.by_status.get("a-future-status"), Some(&1));
        assert!(!open.by_status.contains_key("success"));
    }

    #[test]
    fn a_snapshot_is_a_healthy_copy_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let path = database(dir.path(), &["pending"]);
        let copy = dir.path().join("copy.db");
        snapshot(&path, &copy).unwrap();
        let checked = check(&copy).unwrap();
        assert!(checked.healthy());
        assert_eq!(checked.migrations, 2);
        assert_eq!(checked.latest_migration, Some(2));
        assert_eq!(open_orders(&copy).unwrap().pending, 1);
        assert!(snapshot(&path, &copy).is_err());
    }

    #[test]
    fn a_missing_database_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(open_orders(&dir.path().join("missing.db")).is_err());
        assert!(check(&dir.path().join("missing.db")).is_err());
    }
}
