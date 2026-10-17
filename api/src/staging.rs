//! Produce a reviewable, inert Mostro configuration without touching its daemon volume.
use crate::{config::render_settings, store::Document};
use serde::Serialize;
use std::{
    fs::{self, DirBuilder, File, OpenOptions},
    io::{self, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};
use url::Url;

const MAX_DOCUMENT_BYTES: u64 = 1024 * 1024;

#[derive(Serialize)]
pub struct Staged {
    pub revision: u64,
    pub directory: PathBuf,
    pub state: &'static str,
}

fn ensure_private_dir(path: &Path) -> Result<(), &'static str> {
    match DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err("No se pudo crear el directorio de preparación"),
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| "Directorio no disponible")?;
    if !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 {
        return Err("El directorio de preparación debe ser privado (0700)");
    }
    Ok(())
}

fn write_new(path: &Path, contents: &[u8]) -> Result<(), &'static str> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| "No se pudo crear el archivo preparado")?;
    file.write_all(contents)
        .and_then(|()| file.sync_all())
        .map_err(|_| "No se pudo escribir el archivo preparado")
}

/// A revision is staged once. No identity, macaroon, or daemon database is read.
/// The output is never used by the running Umbrel compose stack.
pub fn stage(root: &Path, lnd_grpc_origin: &str) -> Result<Staged, &'static str> {
    let origin = Url::parse(lnd_grpc_origin).map_err(|_| "Origen LND inválido")?;
    if origin.port().is_none() {
        return Err("El origen LND gRPC debe incluir un puerto explícito");
    }
    let root_metadata = fs::symlink_metadata(root).map_err(|_| "CONFIG_DIR no disponible")?;
    if !root_metadata.is_dir() || root_metadata.permissions().mode() & 0o077 != 0 {
        return Err("CONFIG_DIR debe ser un directorio privado (0700)");
    }
    let document_path = root.join("community.json");
    let metadata = fs::symlink_metadata(&document_path).map_err(|_| "Falta el borrador")?;
    if !metadata.is_file() || metadata.len() > MAX_DOCUMENT_BYTES {
        return Err("Borrador inválido o demasiado grande");
    }
    let document: Document = serde_json::from_slice(
        &fs::read(document_path).map_err(|_| "No se pudo leer el borrador")?,
    )
    .map_err(|_| "Borrador inválido")?;
    let config = document.config.as_ref().ok_or("Borrador vacío")?;
    let settings = render_settings(
        config,
        lnd_grpc_origin,
        &crate::daemon::cert_path(),
        &crate::daemon::macaroon_path(),
    )
    .map_err(|_| "No se pudo generar settings.toml")?;

    let staging = root.join("staging");
    ensure_private_dir(&staging)?;
    let directory = staging.join(format!("revision-{}", document.revision));
    DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .map_err(|_| "La revisión ya está preparada o no se puede crear")?;
    let result = (|| {
        write_new(&directory.join("settings.toml"), settings.as_bytes())?;
        let staged = Staged {
            revision: document.revision,
            directory: directory.clone(),
            state: "staged_only",
        };
        write_new(
            &directory.join("stage.json"),
            &serde_json::to_vec_pretty(&staged).map_err(|_| "No se pudo generar el manifiesto")?,
        )?;
        File::open(&directory)
            .and_then(|file| file.sync_all())
            .map_err(|_| "No se pudo sincronizar la revisión")?;
        Ok(staged)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&directory);
    }
    result
}
