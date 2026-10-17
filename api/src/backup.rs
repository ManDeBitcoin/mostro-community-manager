//! Local, pre-market backup. Ciphertext is never returned by the HTTP API.
use age::{
    Decryptor, Encryptor,
    secrecy::{ExposeSecret, SecretString},
};
use nostr::{FromBech32, Keys, SecretKey, ToBech32};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, DirBuilder, File, OpenOptions},
    io::{self, IsTerminal, Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::RwLock;
use zeroize::Zeroizing;

use crate::{identity, mostro_db, store::Document};

const MAX_PLAINTEXT: u64 = 1024 * 1024;
const MAX_CIPHERTEXT: u64 = 2 * 1024 * 1024;
/// Largest daemon database a backup may hold once decrypted.
const MAX_DATABASE: u64 = 1024 * 1024 * 1024;
/// Prefix of the companion file holding the daemon's database.
const DATABASE_PREFIX: &str = "mostro-db-";

#[derive(Serialize)]
struct BackupWrite<'a> {
    format: &'static str,
    npub: &'a str,
    nsec: &'a str,
    document: &'a Document,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupRead<'a> {
    format: String,
    npub: &'a str,
    nsec: &'a str,
    document: Document,
}

#[derive(Clone, Debug)]
pub struct Summary {
    pub npub: String,
    pub revision: u64,
    pub path: PathBuf,
    /// The daemon's database, encrypted with the same passphrase, when the
    /// node has one (`mostro-db-rev<N>-<ts>.age`, next to `path`).
    pub database_path: Option<PathBuf>,
}

fn read_document(root: &Path) -> Result<Document, &'static str> {
    let path = root.join("community.json");
    let metadata = fs::symlink_metadata(&path).map_err(|_| "Falta el borrador guardado")?;
    if !metadata.is_file() || metadata.len() > MAX_PLAINTEXT {
        return Err("El borrador no es un archivo regular o excede el límite de backup");
    }
    let bytes = fs::read(path).map_err(|_| "No se pudo leer el borrador")?;
    let document: Document =
        serde_json::from_slice(&bytes).map_err(|_| "Borrador guardado inválido")?;
    let config = document
        .config
        .as_ref()
        .ok_or("Borrador sin configuración")?;
    config.validate().map_err(|_| "Borrador inválido")?;
    Ok(document)
}

pub fn ensure_private_dir(path: &Path) -> Result<(), &'static str> {
    match DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err("No se pudo crear el directorio de backups"),
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| "Directorio de backups no disponible")?;
    if !metadata.is_dir() {
        return Err("La ruta de backup no es un directorio regular");
    }
    #[cfg(unix)]
    {
        if metadata.permissions().mode() & 0o077 != 0 {
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
            let refreshed =
                fs::symlink_metadata(path).map_err(|_| "Directorio de backups no disponible")?;
            if refreshed.permissions().mode() & 0o077 != 0 {
                return Err("El directorio de backups debe ser privado (0700)");
            }
        }
    }
    Ok(())
}

fn private_backup_dir(root: &Path) -> Result<PathBuf, &'static str> {
    let path = root.join("backups");
    ensure_private_dir(&path)?;
    Ok(path)
}

fn decrypt_backup(
    path: &Path,
    passphrase: SecretString,
) -> Result<Zeroizing<Vec<u8>>, &'static str> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "Backup no disponible")?;
    if !metadata.is_file() || metadata.len() > MAX_CIPHERTEXT {
        return Err("Archivo de backup inválido o demasiado grande");
    }
    let file = File::open(path).map_err(|_| "No se pudo abrir el backup")?;
    let decryptor = Decryptor::new(file).map_err(|_| "Backup age inválido")?;
    let identity = age::scrypt::Identity::new(passphrase);
    let mut reader = decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .map_err(|_| "No se pudo descifrar el backup")?;
    let mut plaintext = Zeroizing::new(Vec::new());
    reader
        .by_ref()
        .take(MAX_PLAINTEXT + 1)
        .read_to_end(&mut plaintext)
        .map_err(|_| "No se pudo leer el backup descifrado")?;
    if plaintext.len() as u64 > MAX_PLAINTEXT {
        return Err("Contenido de backup demasiado grande");
    }
    Ok(plaintext)
}

fn parse_backup(plaintext: &[u8]) -> Result<BackupRead<'_>, &'static str> {
    let envelope: BackupRead<'_> =
        serde_json::from_slice(plaintext).map_err(|_| "Contenido de backup inválido")?;
    if envelope.format != "mostro-community-pre-market-v1" {
        return Err("Formato de backup no compatible");
    }
    let secret =
        SecretKey::from_bech32(envelope.nsec).map_err(|_| "Identidad de backup inválida")?;
    let npub = Keys::new(secret)
        .public_key()
        .to_bech32()
        .map_err(|_| "Identidad de backup inválida")?;
    if npub != envelope.npub {
        return Err("La identidad del backup no corresponde al npub");
    }
    let config = envelope
        .document
        .config
        .as_ref()
        .ok_or("Backup sin borrador")?;
    config
        .validate()
        .map_err(|_| "Borrador de backup inválido")?;
    Ok(envelope)
}

fn decode_backup(path: &Path, passphrase: SecretString) -> Result<(String, u64), &'static str> {
    let plaintext = decrypt_backup(path, passphrase)?;
    let envelope = parse_backup(&plaintext)?;
    Ok((envelope.npub.to_owned(), envelope.document.revision))
}

pub fn export_to_dir(
    root: &Path,
    target_dir: &Path,
    passphrase: SecretString,
) -> Result<Summary, &'static str> {
    if passphrase.expose_secret().len() < 16 {
        return Err("La frase de cifrado debe tener al menos 16 caracteres");
    }
    let document = read_document(root)?;
    let nsec = identity::read_private(root)?.ok_or("Falta la identidad importada")?;
    let secret = SecretKey::from_bech32(nsec.trim()).map_err(|_| "Identidad inválida")?;
    let npub = Keys::new(secret)
        .public_key()
        .to_bech32()
        .map_err(|_| "Identidad inválida")?;
    let archive = BackupWrite {
        format: "mostro-community-pre-market-v1",
        npub: &npub,
        nsec: nsec.trim(),
        document: &document,
    };
    let plaintext =
        Zeroizing::new(serde_json::to_vec(&archive).map_err(|_| "No se pudo preparar el backup")?);
    if plaintext.len() as u64 > MAX_PLAINTEXT {
        return Err("Contenido de backup demasiado grande");
    }
    ensure_private_dir(target_dir)?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(target_dir).map_err(|_| "No se pudo crear el backup")?;
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|_| "No se pudieron restringir los permisos del backup")?;
    let mut writer = Encryptor::with_user_passphrase(passphrase.clone())
        .wrap_output(temporary.as_file_mut())
        .map_err(|_| "No se pudo iniciar el cifrado")?;
    writer
        .write_all(&plaintext)
        .map_err(|_| "No se pudo cifrar el backup")?;
    writer
        .finish()
        .map_err(|_| "No se pudo cerrar el cifrado")?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| "No se pudo sincronizar el backup")?;
    let (verified_npub, verified_revision) = decode_backup(temporary.path(), passphrase.clone())?;
    if verified_npub != npub || verified_revision != document.revision {
        return Err("El backup cifrado no superó la verificación");
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "Reloj del sistema inválido")?
        .as_secs();
    let path = target_dir.join(format!(
        "pre-market-rev{}-{timestamp}.age",
        document.revision
    ));
    temporary
        .persist_noclobber(&path)
        .map_err(|_| "Ya existe un backup con ese nombre")?;
    File::open(target_dir)
        .and_then(|file| file.sync_all())
        .map_err(|_| "Backup guardado; no se pudo sincronizar el directorio")?;
    // The rules alone cannot bring a market back: the daemon's database holds
    // its orders, users and bonds. It goes in its own file, so the format
    // above stays readable by earlier versions and its size limit holds.
    let database = mostro_db::database_path(root);
    let database_path = if database.is_file() {
        let name = format!("{DATABASE_PREFIX}rev{}-{timestamp}.age", document.revision);
        Some(
            export_database(&database, target_dir, &name, &passphrase).map_err(
                |_| "Respaldo de reglas guardado, pero la base de mostrod no se pudo respaldar",
            )?,
        )
    } else {
        None
    };
    Ok(Summary {
        npub,
        revision: document.revision,
        path,
        database_path,
    })
}

/// Encrypts a consistent copy of the daemon's database into `target_dir/name`,
/// then decrypts it again and compares it with the copy before keeping it.
fn export_database(
    database: &Path,
    target_dir: &Path,
    name: &str,
    passphrase: &SecretString,
) -> Result<PathBuf, &'static str> {
    // Plaintext copies live only in a private folder that is removed after.
    let work = tempfile::Builder::new()
        .prefix(".mostro-db-")
        .tempdir_in(target_dir)
        .map_err(|_| "No se pudo preparar la copia de la base")?;
    fs::set_permissions(work.path(), fs::Permissions::from_mode(0o700))
        .map_err(|_| "No se pudo restringir la copia de la base")?;
    let copy = work.path().join("mostro.db");
    mostro_db::snapshot(database, &copy)?;
    if !mostro_db::check(&copy)?.healthy() {
        return Err("La copia de la base de mostrod no está íntegra");
    }

    let mut encrypted = tempfile::NamedTempFile::new_in(target_dir)
        .map_err(|_| "No se pudo crear el respaldo de la base")?;
    encrypted
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|_| "No se pudieron restringir los permisos del respaldo de la base")?;
    let mut writer = Encryptor::with_user_passphrase(passphrase.clone())
        .wrap_output(encrypted.as_file_mut())
        .map_err(|_| "No se pudo iniciar el cifrado de la base")?;
    io::copy(
        &mut File::open(&copy).map_err(|_| "No se pudo leer la copia de la base")?,
        &mut writer,
    )
    .map_err(|_| "No se pudo cifrar la base")?;
    writer
        .finish()
        .map_err(|_| "No se pudo cerrar el cifrado de la base")?;
    encrypted
        .as_file()
        .sync_all()
        .map_err(|_| "No se pudo sincronizar el respaldo de la base")?;

    let verified = work.path().join("verified.db");
    decrypt_database(encrypted.path(), passphrase.clone(), &verified)?;
    if sha256_file(&verified)? != sha256_file(&copy)? {
        return Err("El respaldo cifrado de la base no superó la verificación");
    }
    let path = target_dir.join(name);
    encrypted
        .persist_noclobber(&path)
        .map_err(|_| "Ya existe un respaldo de la base con ese nombre")?;
    File::open(target_dir)
        .and_then(|file| file.sync_all())
        .map_err(|_| "Respaldo de la base guardado; no se pudo sincronizar el directorio")?;
    Ok(path)
}

fn sha256_file(path: &Path) -> Result<Vec<u8>, &'static str> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    io::copy(
        &mut File::open(path).map_err(|_| "No se pudo leer la base")?,
        &mut hasher,
    )
    .map_err(|_| "No se pudo leer la base")?;
    Ok(hasher.finalize().to_vec())
}

/// Decrypts a database backup into `destination`, a new private file.
fn decrypt_database(
    archive: &Path,
    passphrase: SecretString,
    destination: &Path,
) -> Result<(), &'static str> {
    let metadata =
        fs::symlink_metadata(archive).map_err(|_| "Respaldo de la base no disponible")?;
    if !metadata.is_file() {
        return Err("El respaldo de la base no es un archivo regular");
    }
    let file = File::open(archive).map_err(|_| "No se pudo abrir el respaldo de la base")?;
    let decryptor = Decryptor::new(file).map_err(|_| "Respaldo age inválido")?;
    let identity = age::scrypt::Identity::new(passphrase);
    let mut reader = decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .map_err(|_| "No se pudo descifrar el respaldo de la base")?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(destination)
        .map_err(|_| "No se pudo crear la base restaurada")?;
    let written = io::copy(&mut reader.by_ref().take(MAX_DATABASE + 1), &mut output)
        .map_err(|_| "No se pudo descifrar la base")?;
    if written > MAX_DATABASE {
        return Err("La base del respaldo es demasiado grande");
    }
    output
        .sync_all()
        .map_err(|_| "No se pudo sincronizar la base restaurada")
}

/// Writes the database of a backup to `destination`, which must not exist,
/// after checking it. Nothing running is touched: putting it in place of
/// `active/mostro.db`, with the daemon stopped, is the operator's step.
pub fn restore_database(
    archive: &Path,
    destination: &Path,
    passphrase: SecretString,
) -> Result<mostro_db::DbCheck, &'static str> {
    if !destination.is_absolute() {
        return Err("El destino de restauración debe ser absoluto");
    }
    let parent = destination
        .parent()
        .ok_or("Destino de restauración inválido")?;
    if fs::symlink_metadata(destination).is_ok() {
        return Err("El destino ya existe; no se sobrescribe");
    }
    let work = tempfile::Builder::new()
        .prefix(".mostro-db-restore-")
        .tempdir_in(parent)
        .map_err(|_| "No se pudo preparar la restauración")?;
    let restored = work.path().join("mostro.db");
    decrypt_database(archive, passphrase, &restored)?;
    let checked = mostro_db::check(&restored)?;
    if !checked.healthy() {
        return Err("La base del respaldo no está íntegra");
    }
    if fs::symlink_metadata(destination).is_ok() {
        return Err("El destino ya existe; no se sobrescribe");
    }
    fs::rename(&restored, destination).map_err(|_| "No se pudo publicar la base restaurada")?;
    Ok(checked)
}

pub fn restore_database_interactive(
    archive: &Path,
    destination: &Path,
) -> Result<(), &'static str> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err("La restauración requiere una terminal interactiva local (-it)");
    }
    let passphrase = rpassword::prompt_password("Frase del backup (entrada oculta): ")
        .map_err(|_| "No se pudo leer la frase")?;
    let checked = restore_database(archive, destination, SecretString::from(passphrase))?;
    println!(
        "Base restaurada en {} ({} migraciones, integridad ok). No se aplicó al nodo: con mostrod parado, ponla en lugar de active/mostro.db.",
        destination.display(),
        checked.migrations
    );
    Ok(())
}

pub fn export(root: &Path, passphrase: SecretString) -> Result<Summary, &'static str> {
    let directory = private_backup_dir(root)?;
    export_to_dir(root, &directory, passphrase)
}

pub fn verify(path: &Path, passphrase: SecretString) -> Result<Summary, &'static str> {
    let (npub, revision) = decode_backup(path, passphrase)?;
    Ok(Summary {
        npub,
        revision,
        path: path.to_path_buf(),
        database_path: None,
    })
}

/// Materialize a validated backup into a new private directory. The caller
/// must decide separately when and how to activate it; live data is untouched.
pub fn restore_to_new_dir(
    archive: &Path,
    destination: &Path,
    passphrase: SecretString,
) -> Result<Summary, &'static str> {
    if !destination.is_absolute() {
        return Err("El destino de restauración debe ser absoluto");
    }
    let parent = destination
        .parent()
        .ok_or("Destino de restauración inválido")?;
    let metadata = fs::symlink_metadata(parent).map_err(|_| "Directorio padre no disponible")?;
    if !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 {
        return Err("El directorio padre debe ser privado (0700)");
    }
    match fs::symlink_metadata(destination) {
        Ok(_) => return Err("El destino ya existe; no se sobrescribe"),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err("No se pudo comprobar el destino"),
    }

    let plaintext = decrypt_backup(archive, passphrase)?;
    let envelope = parse_backup(&plaintext)?;
    let staging = tempfile::Builder::new()
        .prefix(".mostro-restore-")
        .tempdir_in(parent)
        .map_err(|_| "No se pudo preparar la restauración")?;
    fs::set_permissions(staging.path(), fs::Permissions::from_mode(0o700))
        .map_err(|_| "No se pudieron restringir los permisos de restauración")?;
    identity::import(staging.path(), envelope.nsec, envelope.npub)?;
    let document = serde_json::to_vec_pretty(&envelope.document)
        .map_err(|_| "No se pudo preparar el borrador restaurado")?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(staging.path().join("community.json"))
        .map_err(|_| "No se pudo crear el borrador restaurado")?;
    file.write_all(&document)
        .map_err(|_| "No se pudo escribir el borrador restaurado")?;
    file.sync_all()
        .map_err(|_| "No se pudo sincronizar el borrador restaurado")?;
    File::open(staging.path())
        .and_then(|dir| dir.sync_all())
        .map_err(|_| "No se pudo sincronizar la restauración")?;
    match fs::symlink_metadata(destination) {
        Ok(_) => return Err("El destino ya existe; no se sobrescribe"),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err("No se pudo comprobar el destino"),
    }
    fs::rename(staging.path(), destination).map_err(|_| "No se pudo publicar la restauración")?;
    File::open(parent)
        .and_then(|dir| dir.sync_all())
        .map_err(|_| "Restauración creada; no se pudo sincronizar el directorio padre")?;
    Ok(Summary {
        npub: envelope.npub.to_owned(),
        revision: envelope.document.revision,
        path: destination.to_path_buf(),
        database_path: None,
    })
}

pub fn export_interactive(root: &Path) -> Result<(), &'static str> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err("El backup requiere una terminal interactiva local (-it)");
    }
    let mut first = Zeroizing::new(
        rpassword::prompt_password("Frase de cifrado (entrada oculta): ")
            .map_err(|_| "No se pudo leer la frase")?,
    );
    let second = Zeroizing::new(
        rpassword::prompt_password("Repite la frase: ").map_err(|_| "No se pudo leer la frase")?,
    );
    if first.len() < 16 || first != second {
        return Err("Las frases deben coincidir y tener al menos 16 caracteres");
    }
    let summary = export(root, SecretString::from(std::mem::take(&mut *first)))?;
    println!(
        "Backup cifrado y verificado: {} (revisión {}, {}). Copia el archivo fuera del servidor.",
        summary.path.display(),
        summary.revision,
        summary.npub
    );
    Ok(())
}

pub fn verify_interactive(path: &Path) -> Result<(), &'static str> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err("La verificación requiere una terminal interactiva local (-it)");
    }
    let passphrase = rpassword::prompt_password("Frase del backup (entrada oculta): ")
        .map_err(|_| "No se pudo leer la frase")?;
    let summary = verify(path, SecretString::from(passphrase))?;
    println!(
        "Backup válido: revisión {}, {}. No se restauraron archivos.",
        summary.revision, summary.npub
    );
    Ok(())
}

pub fn restore_interactive(archive: &Path, destination: &Path) -> Result<(), &'static str> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err("La restauración requiere una terminal interactiva local (-it)");
    }
    let passphrase = rpassword::prompt_password("Frase del backup (entrada oculta): ")
        .map_err(|_| "No se pudo leer la frase")?;
    let summary = restore_to_new_dir(archive, destination, SecretString::from(passphrase))?;
    println!(
        "Restauración aislada creada: {} (revisión {}, {}). No se aplicó a la instalación activa.",
        summary.path.display(),
        summary.revision,
        summary.npub
    );
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupEntryInfo {
    pub filename: String,
    pub path: String,
    pub size_bytes: u64,
    pub modified_timestamp: u64,
    /// `rules` (identity and rules) or `database` (the daemon's database).
    #[serde(default)]
    pub kind: String,
}

fn backup_kind(filename: &str) -> &'static str {
    if filename.starts_with(DATABASE_PREFIX) {
        "database"
    } else {
        "rules"
    }
}

/// The files of one backup share the timestamp at the end of their names
/// (`…-<unix seconds>.age`); a file named otherwise is a backup of its own.
fn backup_group(path: &Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    name.strip_suffix(".age")
        .and_then(|stem| stem.rsplit_once('-'))
        .map(|(_, timestamp)| timestamp)
        .filter(|timestamp| !timestamp.is_empty() && timestamp.bytes().all(|b| b.is_ascii_digit()))
        .map(str::to_owned)
        .unwrap_or(name)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AutoBackupState {
    pub enabled: bool,
    pub target_dir: String,
    pub interval_secs: u64,
    pub retention_count: usize,
    pub last_run_timestamp: Option<u64>,
    pub last_run_success: Option<bool>,
    pub last_error: Option<String>,
    pub last_backup_path: Option<String>,
    pub backups: Vec<BackupEntryInfo>,
}

impl Default for AutoBackupState {
    fn default() -> Self {
        Self {
            enabled: false,
            target_dir: String::new(),
            interval_secs: 86400,
            retention_count: 7,
            last_run_timestamp: None,
            last_run_success: None,
            last_error: None,
            last_backup_path: None,
            backups: Vec::new(),
        }
    }
}

pub fn list_backups(target_dir: &Path) -> Vec<BackupEntryInfo> {
    let mut list = Vec::new();
    if let Ok(entries) = fs::read_dir(target_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && path.extension().and_then(|e| e.to_str()) == Some("age")
                && !path.is_symlink()
                && let Ok(meta) = entry.metadata()
            {
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let filename = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                list.push(BackupEntryInfo {
                    kind: backup_kind(&filename).to_string(),
                    filename,
                    path: path.to_string_lossy().to_string(),
                    size_bytes: meta.len(),
                    modified_timestamp: mtime,
                });
            }
        }
    }
    list.sort_by_key(|entry| std::cmp::Reverse(entry.modified_timestamp));
    list
}

pub fn apply_retention_policy(
    target_dir: &Path,
    keep_count: usize,
) -> Result<Vec<PathBuf>, &'static str> {
    let metadata =
        fs::symlink_metadata(target_dir).map_err(|_| "Directorio de backup no encontrado")?;
    if !metadata.is_dir() {
        return Err("Ruta de retención no es un directorio");
    }
    let keep = keep_count.max(1);
    let entries = fs::read_dir(target_dir).map_err(|_| "No se pudo leer directorio de backup")?;
    let mut backups: Vec<(PathBuf, SystemTime)> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && path.extension().and_then(|e| e.to_str()) == Some("age")
            && !path.is_symlink()
        {
            let mtime = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(UNIX_EPOCH);
            backups.push((path, mtime));
        }
    }

    // A backup is the rules file plus, when the node has one, its database
    // file: keep or remove them together, newest backups first.
    let mut groups: Vec<(String, SystemTime, Vec<PathBuf>)> = Vec::new();
    for (path, mtime) in backups {
        let group = backup_group(&path);
        match groups.iter_mut().find(|(name, _, _)| *name == group) {
            Some((_, newest, paths)) => {
                *newest = (*newest).max(mtime);
                paths.push(path);
            }
            None => groups.push((group, mtime, vec![path])),
        }
    }
    groups.sort_by_key(|group| std::cmp::Reverse(group.1));

    let mut removed = Vec::new();
    for (_, _, paths) in groups.iter().skip(keep) {
        for path in paths {
            if fs::remove_file(path).is_ok() {
                removed.push(path.clone());
            }
        }
    }

    Ok(removed)
}

pub fn run_auto_backup_cycle(
    root: &Path,
    target_dir: &Path,
    retention_count: usize,
    passphrase: SecretString,
) -> Result<Summary, &'static str> {
    let summary = export_to_dir(root, target_dir, passphrase)?;
    let _ = apply_retention_policy(target_dir, retention_count);
    Ok(summary)
}

pub async fn auto_backup_worker(
    root: PathBuf,
    target_dir: PathBuf,
    interval: std::time::Duration,
    retention_count: usize,
    passphrase_opt: Option<SecretString>,
    state: Arc<RwLock<AutoBackupState>>,
    notifications: Arc<crate::notifications::NotificationHub>,
) {
    let enabled = passphrase_opt.is_some();
    {
        let mut st = state.write().await;
        st.enabled = enabled;
        st.target_dir = target_dir.display().to_string();
        st.interval_secs = interval.as_secs();
        st.retention_count = retention_count;
        st.backups = list_backups(&target_dir);
        if !enabled {
            st.last_error = Some("Frase de cifrado no configurada (BACKUP_PASSPHRASE)".into());
        }
    }

    let Some(passphrase) = passphrase_opt else {
        return;
    };

    // Initial small delay
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    loop {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        match export_to_dir(&root, &target_dir, passphrase.clone()) {
            Ok(summary) => {
                let _ = apply_retention_policy(&target_dir, retention_count);
                let backups = list_backups(&target_dir);
                {
                    let mut st = state.write().await;
                    st.last_run_timestamp = Some(now);
                    st.last_run_success = Some(true);
                    st.last_error = None;
                    st.last_backup_path = Some(summary.path.display().to_string());
                    st.backups = backups;
                }
                notifications
                    .publish(crate::notifications::Notification::backup_alert(
                        "Respaldo automático completado",
                        &format!("Respaldo guardado en {}", summary.path.display()),
                        true,
                        Some(serde_json::json!({
                            "path": summary.path.display().to_string(),
                            "revision": summary.revision,
                            "npub": summary.npub
                        })),
                    ))
                    .await;
            }
            Err(e) => {
                let backups = list_backups(&target_dir);
                {
                    let mut st = state.write().await;
                    st.last_run_timestamp = Some(now);
                    st.last_run_success = Some(false);
                    st.last_error = Some(e.to_string());
                    st.backups = backups;
                }
                notifications
                    .publish(crate::notifications::Notification::backup_alert(
                        "El respaldo automático falló",
                        &format!("Error al exportar respaldo: {e}"),
                        false,
                        Some(serde_json::json!({"error": e})),
                    ))
                    .await;
            }
        }

        tokio::time::sleep(interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Configuration, store::Store};
    use nostr::ToBech32;

    #[test]
    fn encrypted_backup_round_trip_and_wrong_password() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let mut store = Store::open(root.path().to_path_buf()).unwrap();
        let config: Configuration =
            serde_json::from_str(include_str!("../tests/fixtures/community.json")).unwrap();
        store.save(config).unwrap();
        let keys = Keys::new(SecretKey::from_slice(&[7; 32]).unwrap());
        let nsec = keys.secret_key().to_bech32().unwrap();
        let npub = keys.public_key().to_bech32().unwrap();
        identity::import(root.path(), &nsec, &npub).unwrap();
        let exported = export(
            root.path(),
            SecretString::from("test-passphrase-not-for-use".to_owned()),
        )
        .unwrap();
        assert_eq!(exported.npub, npub);
        assert_eq!(exported.revision, 1);
        let ciphertext = fs::read(&exported.path).unwrap();
        assert!(
            !ciphertext
                .windows(nsec.len())
                .any(|part| part == nsec.as_bytes())
        );
        assert_eq!(
            fs::metadata(&exported.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(root.path().join("backups"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert!(
            verify(
                &exported.path,
                SecretString::from("wrong-password".to_owned())
            )
            .is_err()
        );
        let verified = verify(
            &exported.path,
            SecretString::from("test-passphrase-not-for-use".to_owned()),
        )
        .unwrap();
        assert_eq!(verified.npub, npub);
        assert_eq!(verified.revision, 1);
        let restored_path = root.path().join("restored");
        let restored = restore_to_new_dir(
            &exported.path,
            &restored_path,
            SecretString::from("test-passphrase-not-for-use".to_owned()),
        )
        .unwrap();
        assert_eq!(restored.npub, npub);
        assert_eq!(restored.revision, 1);
        assert_eq!(
            identity::inspect(&restored_path).unwrap(),
            Some(npub.clone())
        );
        assert_eq!(
            Store::open(restored_path.clone())
                .unwrap()
                .document
                .revision,
            1
        );
        assert_eq!(
            fs::metadata(&restored_path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        fs::write(restored_path.join("marker"), "keep").unwrap();
        assert!(
            restore_to_new_dir(
                &exported.path,
                &restored_path,
                SecretString::from("test-passphrase-not-for-use".to_owned()),
            )
            .is_err()
        );
        assert_eq!(
            fs::read_to_string(restored_path.join("marker")).unwrap(),
            "keep"
        );
        let mut tampered = ciphertext;
        let last = tampered.last_mut().unwrap();
        *last ^= 1;
        let tampered_path = root.path().join("tampered.age");
        fs::write(&tampered_path, tampered).unwrap();
        assert!(
            verify(
                &tampered_path,
                SecretString::from("test-passphrase-not-for-use".to_owned())
            )
            .is_err()
        );
    }

    #[test]
    fn test_export_to_custom_dir_and_retention() {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let mut store = Store::open(root.path().to_path_buf()).unwrap();
        let config: Configuration =
            serde_json::from_str(include_str!("../tests/fixtures/community.json")).unwrap();
        store.save(config).unwrap();
        let keys = Keys::new(SecretKey::from_slice(&[7; 32]).unwrap());
        let nsec = keys.secret_key().to_bech32().unwrap();
        let npub = keys.public_key().to_bech32().unwrap();
        identity::import(root.path(), &nsec, &npub).unwrap();

        let offsite = tempfile::tempdir().unwrap();
        let offsite_path = offsite.path().join("backup");

        let summary1 = export_to_dir(
            root.path(),
            &offsite_path,
            SecretString::from("test-passphrase-not-for-use".to_owned()),
        )
        .unwrap();
        assert_eq!(summary1.npub, npub);
        assert!(summary1.path.exists());

        // Check directory permission is 0700
        assert_eq!(
            fs::metadata(&offsite_path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        // Check file permission is 0600
        assert_eq!(
            fs::metadata(&summary1.path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        // Create a few synthetic backup files with different timestamps
        let f2 = offsite_path.join("pre-market-rev1-1000.age");
        let f3 = offsite_path.join("pre-market-rev1-2000.age");
        let f4 = offsite_path.join("pre-market-rev1-3000.age");
        fs::copy(&summary1.path, &f2).unwrap();
        fs::copy(&summary1.path, &f3).unwrap();
        fs::copy(&summary1.path, &f4).unwrap();

        let all_backups = list_backups(&offsite_path);
        assert_eq!(all_backups.len(), 4);

        // Apply retention: keep 2
        let removed = apply_retention_policy(&offsite_path, 2).unwrap();
        assert_eq!(removed.len(), 2);

        let remaining = list_backups(&offsite_path);
        assert_eq!(remaining.len(), 2);
    }

    #[test]
    fn test_export_to_dir_short_passphrase_rejected() {
        let root = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let err = export_to_dir(
            root.path(),
            target.path(),
            SecretString::from("too-short".to_owned()),
        )
        .unwrap_err();
        assert!(err.contains("al menos 16 caracteres"));
    }
}
