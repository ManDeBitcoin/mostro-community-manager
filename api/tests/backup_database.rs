//! The daemon's database travels with the rules in every backup.
use age::secrecy::SecretString;
use mostro_community_api::{
    backup::{apply_retention_policy, export_to_dir, list_backups, restore_database},
    config::Configuration,
    identity, mostro_db,
    store::Store,
};
use nostr::{Keys, SecretKey, ToBech32};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    time::{Duration, SystemTime},
};

fn node_with_database(root: &Path) {
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
    let mut store = Store::open(root.to_path_buf()).unwrap();
    let config: Configuration =
        serde_json::from_str(include_str!("fixtures/community.json")).unwrap();
    store.save(config).unwrap();
    let keys = Keys::new(SecretKey::from_slice(&[7; 32]).unwrap());
    identity::import(
        root,
        &keys.secret_key().to_bech32().unwrap(),
        &keys.public_key().to_bech32().unwrap(),
    )
    .unwrap();
    fs::create_dir(root.join("active")).unwrap();
    let db = rusqlite::Connection::open(mostro_db::database_path(root)).unwrap();
    db.execute_batch(
        "create table _sqlx_migrations (version integer primary key, success boolean not null);
         insert into _sqlx_migrations values (20260101000000, 1), (20260201000000, 1);
         create table orders (id text primary key, status text not null);
         insert into orders values ('a', 'pending'), ('b', 'success');",
    )
    .unwrap();
}

fn passphrase() -> SecretString {
    SecretString::from("frase-de-prueba-larga-0123456789".to_string())
}

fn set_age(path: &Path, seconds: u64) {
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
        .unwrap();
}

#[test]
fn a_backup_carries_the_daemon_database_and_restores_it() {
    let root = tempfile::tempdir().unwrap();
    node_with_database(root.path());
    let target = tempfile::tempdir().unwrap();
    let dir = target.path().join("backup");

    let summary = export_to_dir(root.path(), &dir, passphrase()).unwrap();
    let database = summary.database_path.expect("the node has a database");
    let rules_name = summary
        .path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let database_name = database.file_name().unwrap().to_string_lossy().to_string();
    assert!(database_name.starts_with("mostro-db-rev1-"));
    assert_eq!(
        rules_name.rsplit_once('-').unwrap().1,
        database_name.rsplit_once('-').unwrap().1,
        "both files of a backup share its timestamp"
    );
    assert_eq!(
        fs::metadata(&database).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let kinds: Vec<String> = list_backups(&dir).into_iter().map(|b| b.kind).collect();
    assert!(kinds.contains(&"rules".to_string()) && kinds.contains(&"database".to_string()));
    // No plaintext copy is left behind.
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);

    let restored = target.path().join("restaurada.db");
    let checked = restore_database(&database, &restored, passphrase()).unwrap();
    assert_eq!(checked.migrations, 2);
    assert_eq!(mostro_db::open_orders(&restored).unwrap().pending, 1);

    // Never over an existing file, never with another passphrase.
    assert!(restore_database(&database, &restored, passphrase()).is_err());
    let other = target.path().join("otra.db");
    assert!(
        restore_database(
            &database,
            &other,
            SecretString::from("otra-frase-de-prueba-0123456789".to_string())
        )
        .is_err()
    );
    assert!(!other.exists());
}

#[test]
fn a_node_without_a_database_backs_up_its_rules_only() {
    let root = tempfile::tempdir().unwrap();
    node_with_database(root.path());
    fs::remove_file(mostro_db::database_path(root.path())).unwrap();
    let target = tempfile::tempdir().unwrap();
    let summary = export_to_dir(root.path(), &target.path().join("backup"), passphrase()).unwrap();
    assert!(summary.database_path.is_none());
}

#[test]
fn retention_keeps_or_removes_both_files_of_a_backup() {
    let root = tempfile::tempdir().unwrap();
    node_with_database(root.path());
    let target = tempfile::tempdir().unwrap();
    let dir = target.path().join("backup");
    let summary = export_to_dir(root.path(), &dir, passphrase()).unwrap();
    let database = summary.database_path.unwrap();

    // An older backup: same pair of files, an earlier timestamp.
    let old_rules = dir.join("pre-market-rev1-1000.age");
    let old_database = dir.join("mostro-db-rev1-1000.age");
    fs::copy(&summary.path, &old_rules).unwrap();
    fs::copy(&database, &old_database).unwrap();
    set_age(&old_rules, 1_000);
    set_age(&old_database, 1_000);

    let removed = apply_retention_policy(&dir, 1).unwrap();
    assert_eq!(removed.len(), 2);
    assert!(!old_rules.exists() && !old_database.exists());
    assert!(summary.path.exists() && database.exists());
}
