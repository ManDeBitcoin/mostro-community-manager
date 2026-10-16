//! Daemon orchestration, runtime validation and safe activation.
//! Ensures Mostro daemon only starts when all cryptographic and network prerequisites are satisfied.
use crate::{
    adapters::Integrations,
    config::render_settings,
    identity,
    orders::{NODE_INFO_FRESH_SECS, NodeInfo},
    store::Document,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, DirBuilder, File, OpenOptions},
    io::{self, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use url::Url;

const CERT_PATH: &str = "/lnd/tls.cert";
const MACAROON_PATH: &str = "/lnd/mostro.macaroon";
const MAX_DOCUMENT_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DaemonState {
    Unconfigured,
    ConfiguredStandby,
    ActiveReady,
    ActiveRunning,
}

/// mostrod release this package pins and ships (see `config/versions.json`).
pub const MOSTRO_VERSION: &str = "0.20.0";
pub const PROTOCOL_VERSION: u32 = 2;

/// Where a reported daemon version comes from, most trustworthy first.
pub const VERSION_SOURCE_ANNOUNCED: &str = "announced";
pub const VERSION_SOURCE_BINARY: &str = "binary";
pub const VERSION_SOURCE_PINNED: &str = "pinned";

fn version_from_output(stdout: &[u8]) -> Option<String> {
    // `mostrod --version` prints "mostro p2p <version>" after terminal control codes.
    String::from_utf8_lossy(stdout)
        .split_whitespace()
        .last()
        .filter(|v| v.len() <= 32 && v.bytes().next().is_some_and(|b| b.is_ascii_digit()))
        .map(str::to_string)
}

/// Version of the mostrod binary shipped next to this API, and how it was
/// obtained. This describes the package, not the process that is running:
/// in Umbrel the daemon lives in another container of the same image.
pub fn packaged_mostrod_version() -> (String, &'static str) {
    for bin_path in ["/usr/local/bin/mostrod", "mostrod"] {
        if let Ok(output) = std::process::Command::new(bin_path)
            .env("TERM", "xterm")
            .arg("--version")
            .output()
            && output.status.success()
            && let Some(version) = version_from_output(&output.stdout)
        {
            return (version, VERSION_SOURCE_BINARY);
        }
    }
    (MOSTRO_VERSION.to_string(), VERSION_SOURCE_PINNED)
}

pub fn detect_mostrod_version() -> String {
    packaged_mostrod_version().0
}

/// Last unexpected exit of mostrod, recorded by the container entrypoint.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LastExit {
    pub at_unix: u64,
    pub code: i32,
    pub uptime_secs: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DaemonReport {
    pub state: DaemonState,
    pub identity_present: bool,
    pub npub: Option<String>,
    pub draft_revision: Option<u64>,
    pub active_revision: Option<u64>,
    pub active_settings_hash: Option<String>,
    pub active_settings_path: Option<String>,
    /// `None` when the panel could not read LND: unknown is not zero.
    pub lnd_channel_count: Option<u64>,
    pub lnd_synced: Option<bool>,
    pub can_activate: bool,
    /// What the operator should know, as plain sentences.
    pub warnings: Vec<String>,
    /// The same warnings, each with a stable code, so that a client can decide
    /// where to send the operator without parsing the sentence.
    #[serde(default)]
    pub notices: Vec<DaemonNotice>,
    /// Seconds the current mostrod process has been running, when known.
    #[serde(default)]
    pub running_for_secs: Option<u64>,
    /// Best known daemon version: the one the daemon announces on the relays
    /// when that announcement is recent, otherwise the packaged binary's.
    pub mostro_version: String,
    /// `announced`, `binary` or `pinned`: what `mostro_version` is based on.
    #[serde(default)]
    pub version_source: String,
    /// Version of the mostrod binary shipped in this package.
    #[serde(default)]
    pub packaged_version: String,
    pub protocol_version: u32,
    /// The daemon's own kind 38385 event, as last seen on the relays.
    #[serde(default)]
    pub announced: Option<NodeInfo>,
    #[serde(default)]
    pub announced_age_secs: Option<u64>,
    /// True while the announcement is recent enough to prove a live daemon:
    /// published in the last minutes and, when the age of the running process
    /// is known, by that process and not by the one before it.
    #[serde(default)]
    pub announced_fresh: bool,
    /// The announcement is recent, but older than the process that is running
    /// now: the previous process published it.
    #[serde(default)]
    pub announced_before_start: bool,
    #[serde(default)]
    pub last_exit: Option<LastExit>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonNotice {
    pub code: String,
    pub text: String,
}

impl DaemonNotice {
    fn new(code: &str, text: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            text: text.into(),
        }
    }
}

/// Every code `report` can emit. The panel maps each one to a page, and
/// `scripts/tests` checks that it knows them all.
pub const NOTICE_CODES: &[&str] = &[
    "identity_missing",
    "rules_missing",
    "lnd_unconfigured",
    "lnd_unreadable",
    "lnd_no_channels",
    "lnd_channels_unknown",
    "lnd_not_synced",
    "daemon_crash_loop",
    "daemon_unexpected_exit",
    "daemon_not_verified",
    "announcement_stale",
    "announcement_without_daemon",
    "version_mismatch",
    "rules_not_applied",
    "routing_fee_zero",
    "pow_first_contact_above_base",
];

/// Slack for telling whether an announcement is older than the running
/// process: both ages are whole seconds, read at slightly different moments.
const ANNOUNCEMENT_START_MARGIN_SECS: u64 = 5;

/// How long an exit the daemon was not asked for stays among the notices.
const UNEXPECTED_EXIT_NEWS_SECS: u64 = 3_600;

/// What the age of the node's last announcement says about the daemon this
/// panel sees. Every answer of the API that calls a market open goes through
/// here, so that two of them cannot disagree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnnouncementAge {
    /// Published in the last minutes: what an app sees as an active node.
    pub recent: bool,
    /// Recent, but older than the process that is running now.
    pub before_start: bool,
}

impl AnnouncementAge {
    /// `running_for_secs` is `None` when no daemon is seen running or its
    /// start is not known. What is not known cannot rule an announcement out.
    pub fn judge(age_secs: Option<u64>, running_for_secs: Option<u64>) -> Self {
        let recent = age_secs.is_some_and(|age| age <= NODE_INFO_FRESH_SECS);
        // The process that ran before this one may have announced itself
        // minutes ago. That event says nothing about the process running now:
        // neither that it is serving clients nor which version it is.
        let before_start = recent
            && age_secs
                .zip(running_for_secs)
                .is_some_and(|(age, running)| {
                    age > running.saturating_add(ANNOUNCEMENT_START_MARGIN_SECS)
                });
        Self {
            recent,
            before_start,
        }
    }

    /// Recent enough to prove a live daemon, and published by it.
    pub fn fresh(self) -> bool {
        self.recent && !self.before_start
    }
}

/// `código 101`, or `código 137 (señal 9)`: the supervisor's shell reports a
/// process ended by a signal as 128 plus the number of the signal.
fn exit_code_text(code: i32) -> String {
    match code {
        129..=192 => format!("código {code} (señal {})", code - 128),
        _ => format!("código {code}"),
    }
}

/// `45 s`, `12 min`, `3 h`, `2 d`: an age in the unit a person would say it in.
fn human_age(secs: u64) -> String {
    if secs < 90 {
        format!("{secs} s")
    } else if secs < 5_400 {
        format!("{} min", (secs + 30) / 60)
    } else if secs < 172_800 {
        format!("{} h", (secs + 1_800) / 3_600)
    } else {
        format!("{} d", (secs + 43_200) / 86_400)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActiveStatus {
    pub revision: u64,
    pub settings_sha256: String,
    pub lnd_grpc_origin: String,
    pub activated_at_unix: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActivationResult {
    pub revision: u64,
    pub settings_path: PathBuf,
    pub settings_sha256: String,
    pub activated_at_unix: u64,
    /// False when the daemon already reads the active file the way it would
    /// read the rendered settings.
    #[serde(default)]
    pub settings_changed: bool,
}

fn ensure_private_dir(path: &Path) -> Result<(), &'static str> {
    match DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err("No se pudo crear el directorio de activación"),
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| "Directorio no disponible")?;
    if !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 {
        return Err("El directorio debe ser privado (0700) y no un enlace");
    }
    Ok(())
}

/// Replaces `path` atomically: the daemon never reads a half-written file.
fn write_private(path: &Path, contents: &[u8]) -> Result<(), &'static str> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Ruta de configuración activa inválida")?;
    let temp = path.with_file_name(format!(".{file_name}.tmp"));
    let _ = fs::remove_file(&temp);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)
        .map_err(|_| "No se pudo crear el archivo de configuración activa")?;
    let written = file
        .write_all(contents)
        .and_then(|()| file.sync_all())
        .and_then(|()| fs::rename(&temp, path));
    if written.is_err() {
        let _ = fs::remove_file(&temp);
        return Err("No se pudo escribir el archivo de configuración activa");
    }
    Ok(())
}

/// Whether mostrod would read both files the same way. A file written by an
/// earlier version of this panel holds the profile fields as they were typed,
/// also when empty; they are now trimmed, and left out when empty.
fn same_settings(on_disk: &[u8], rendered: &str) -> bool {
    fn parsed(text: &str) -> Option<toml::Value> {
        let mut doc: toml::Value = toml::from_str(text).ok()?;
        if let Some(mostro) = doc.get_mut("mostro").and_then(toml::Value::as_table_mut) {
            for key in ["name", "about", "website", "picture"] {
                let Some(value) = mostro.get(key).and_then(toml::Value::as_str) else {
                    continue;
                };
                let trimmed = value.trim().to_string();
                if trimmed.is_empty() {
                    mostro.remove(key);
                } else {
                    mostro.insert(key.into(), trimmed.into());
                }
            }
        }
        Some(doc)
    }
    std::str::from_utf8(on_disk)
        .ok()
        .and_then(parsed)
        .is_some_and(|current| Some(current) == parsed(rendered))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn read_last_exit(active_dir: &Path) -> Option<LastExit> {
    let text = fs::read_to_string(active_dir.join("mostro.last_exit")).ok()?;
    let mut fields = text.split_whitespace();
    Some(LastExit {
        at_unix: fields.next()?.parse().ok()?,
        code: fields.next()?.parse().ok()?,
        uptime_secs: fields.next()?.parse().ok()?,
    })
}

/// Seconds since the entrypoint started the current mostrod process.
fn current_run_secs(active_dir: &Path) -> Option<u64> {
    fs::metadata(active_dir.join("mostro.pid"))
        .ok()?
        .modified()
        .ok()?
        .elapsed()
        .ok()
        .map(|elapsed| elapsed.as_secs())
}

/// What the files the supervisor keeps in `active/` say about the process.
struct Runtime {
    last_exit: Option<LastExit>,
    run_secs: Option<u64>,
    /// The daemon keeps dying right after it starts: it is not running, even
    /// though the entrypoint has just respawned it.
    crash_looping: bool,
    /// Settings are active and a daemon that is not crash looping is alive.
    running: bool,
}

impl Runtime {
    fn read(active_dir: &Path, now: u64) -> Self {
        let last_exit = read_last_exit(active_dir);
        let run_secs = current_run_secs(active_dir);
        let crash_looping = last_exit.as_ref().is_some_and(|exit| {
            now.saturating_sub(exit.at_unix) < 120
                && exit.uptime_secs < 60
                && run_secs.is_none_or(|secs| secs < 30)
        });
        let running = active_dir.join("settings.toml").is_file()
            && is_mostrod_running(active_dir)
            && !crash_looping;
        Self {
            last_exit,
            run_secs,
            crash_looping,
            running,
        }
    }

    fn running_for_secs(&self) -> Option<u64> {
        self.run_secs.filter(|_| self.running)
    }
}

/// Seconds the daemon of this instance has been running. `None` when it is
/// not seen running, or when its start is not known.
pub fn running_for_secs(root: &Path) -> Option<u64> {
    Runtime::read(&root.join("active"), unix_now()).running_for_secs()
}

pub async fn report(root: &Path, integrations: &Integrations) -> DaemonReport {
    report_with_node_info(root, integrations, None).await
}

/// `node_info` is the daemon's own kind 38385 event from the relay monitor.
/// It is the only evidence that a daemon with this identity is really alive.
pub async fn report_with_node_info(
    root: &Path,
    integrations: &Integrations,
    node_info: Option<NodeInfo>,
) -> DaemonReport {
    let npub = identity::inspect(root).unwrap_or(None);
    let identity_present = npub.is_some();

    let document_path = root.join("community.json");
    let draft_doc: Option<Document> = fs::read(&document_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    let draft_revision = draft_doc.as_ref().map(|d| d.revision);
    let max_routing_fee_bps = draft_doc
        .as_ref()
        .and_then(|d| d.config.as_ref())
        .map(|c| c.market.max_routing_fee_bps);
    let first_contact_pow_above_base = draft_doc
        .as_ref()
        .and_then(|d| d.config.as_ref())
        .is_some_and(|c| c.safety.pow_first_contact > c.safety.pow);

    let active_dir = root.join("active");
    let settings_file = active_dir.join("settings.toml");
    let status_file = active_dir.join("status.json");

    let (active_revision, active_settings_hash, active_settings_path) = if settings_file.is_file() {
        let status: Option<ActiveStatus> = fs::read(&status_file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        let hash = fs::read(&settings_file).ok().map(|bytes| {
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            format!("{:x}", hasher.finalize())
        });
        (
            status.map(|s| s.revision),
            hash,
            Some(settings_file.display().to_string()),
        )
    } else {
        (None, None, None)
    };

    let lnd_probe = integrations.lightning().await;
    let lnd_channel_count = lnd_probe
        .get("num_active_channels")
        .and_then(|v| v.as_u64());
    let lnd_synced = lnd_probe.get("synced_to_chain").and_then(|v| v.as_bool());

    let mut notices = Vec::new();
    if !identity_present {
        notices.push(DaemonNotice::new(
            "identity_missing",
            "Falta la identidad del nodo: crea o importa su clave privada Nostr (nsec).",
        ));
    }
    if draft_revision.is_none() {
        notices.push(DaemonNotice::new(
            "rules_missing",
            "Faltan las reglas de la comunidad: aún no hay ninguna configuración guardada.",
        ));
    }
    notices.extend(lnd_notices(&lnd_probe));

    let now = unix_now();
    let runtime = Runtime::read(&active_dir, now);
    let running_for_secs = runtime.running_for_secs();
    let Runtime {
        last_exit,
        run_secs,
        crash_looping,
        running,
    } = runtime;

    let can_activate = identity_present && draft_revision.is_some();
    let state = if active_settings_path.is_some() {
        if running {
            DaemonState::ActiveRunning
        } else {
            DaemonState::ActiveReady
        }
    } else if can_activate {
        DaemonState::ConfiguredStandby
    } else {
        DaemonState::Unconfigured
    };

    let announced_age_secs = node_info
        .as_ref()
        .map(|info| now.saturating_sub(info.created_at));
    let announcement = AnnouncementAge::judge(announced_age_secs, running_for_secs);
    let announced_before_start = announcement.before_start;
    let announced_fresh = announcement.fresh();
    let announced_version = node_info
        .as_ref()
        .filter(|_| announced_fresh)
        .and_then(|info| info.mostro_version.clone());
    let (packaged_version, packaged_source) = packaged_mostrod_version();

    if crash_looping && let Some(exit) = &last_exit {
        notices.push(DaemonNotice::new(
            "daemon_crash_loop",
            format!(
                "El daemon terminó con código {} a los {} de arrancar y se está reiniciando. Revisa los registros del contenedor mostro: suele deberse a LND, a los relays o a la configuración.",
                exit.code,
                human_age(exit.uptime_secs)
            ),
        ));
    }
    // The supervisor restarts a daemon that ends on its own, so once the new
    // process is up a single exit leaves no other trace in this report.
    if !crash_looping
        && active_settings_path.is_some()
        && let Some(exit) = &last_exit
        && now.saturating_sub(exit.at_unix) < UNEXPECTED_EXIT_NEWS_SECS
    {
        notices.push(DaemonNotice::new(
            "daemon_unexpected_exit",
            format!(
                "El daemon terminó de forma inesperada hace {}, con {} y tras {} en marcha. {} Revisa los registros del contenedor mostro para ver la causa.",
                human_age(now.saturating_sub(exit.at_unix)),
                exit_code_text(exit.code),
                human_age(exit.uptime_secs),
                if state == DaemonState::ActiveRunning {
                    "El supervisor lo volvió a arrancar."
                } else {
                    "El supervisor lo vuelve a intentar con una pausa entre intentos."
                }
            ),
        ));
    }
    if state == DaemonState::ActiveReady && !crash_looping {
        notices.push(DaemonNotice::new("daemon_not_verified", "Configuración guardada, pero ejecución del daemon sin verificar. En Umbrel el daemon está en otro contenedor; guardar settings.toml no confirma su arranque ni la versión que reciben los clientes."));
    }
    if state == DaemonState::ActiveRunning
        && !announced_fresh
        && run_secs.is_some_and(|secs| secs > NODE_INFO_FRESH_SECS)
    {
        notices.push(DaemonNotice::new("announcement_stale", "El daemon figura en ejecución, pero su evento de información (kind 38385) no aparece actualizado en los relays configurados. Los clientes no pueden confirmar que el nodo está activo: revisa los relays y los registros."));
    }
    if state != DaemonState::ActiveRunning
        && announced_fresh
        && let Some(age) = announced_age_secs
    {
        notices.push(DaemonNotice::new(
            "announcement_without_daemon",
            format!(
                "El nodo anunció su información en los relays hace {}, pero este panel no ve el daemon en ejecución. Es normal durante unos minutos después de detenerlo. Si el anuncio se sigue renovando, otra instancia de Mostro usa esta misma clave: no actives una segunda.",
                human_age(age)
            ),
        ));
    }
    if let Some(announced) = &announced_version
        && state == DaemonState::ActiveRunning
        && *announced != packaged_version
    {
        notices.push(DaemonNotice::new(
            "version_mismatch",
            format!(
                "El daemon anuncia la versión {announced} y este paquete incluye la {packaged_version}. Reinicia la aplicación para aplicar la actualización o comprueba que no responda otra instancia."
            ),
        ));
    }
    if active_revision.is_some() && active_revision != draft_revision {
        notices.push(DaemonNotice::new(
            "rules_not_applied",
            "Hay reglas guardadas que el nodo aún no tiene aplicadas.",
        ));
    }
    if max_routing_fee_bps == Some(0) {
        notices.push(DaemonNotice::new("routing_fee_zero", "La comisión máxima de enrutamiento es 0: Mostro solo podrá pagar a los compradores por rutas sin comisión y los pagos pueden quedar reintentándose. El valor por defecto de Mostro es 0,2 %."));
    }
    if first_contact_pow_above_base {
        notices.push(DaemonNotice::new("pow_first_contact_above_base", "La prueba de trabajo de la primera conversación es mayor que la general: el daemon descarta sin respuesta las órdenes, tomas, valoraciones y acciones de mediación de los clientes que no la calculan. Comprueba que tus clientes leen pow_first_contact antes de activarla."));
    }

    let (mostro_version, version_source) = match announced_version {
        Some(version) => (version, VERSION_SOURCE_ANNOUNCED),
        None => (packaged_version.clone(), packaged_source),
    };

    DaemonReport {
        state,
        identity_present,
        npub,
        draft_revision,
        active_revision,
        active_settings_hash,
        active_settings_path,
        lnd_channel_count,
        lnd_synced,
        can_activate,
        warnings: notices.iter().map(|notice| notice.text.clone()).collect(),
        notices,
        running_for_secs,
        mostro_version,
        version_source: version_source.to_string(),
        packaged_version,
        protocol_version: PROTOCOL_VERSION,
        announced: node_info,
        announced_age_secs,
        announced_fresh,
        announced_before_start,
        last_exit,
    }
}

/// What the read-only LND probe lets the panel say. A probe that could not
/// read LND knows nothing about its channels: it must not report "0 channels".
pub fn lnd_notices(probe: &serde_json::Value) -> Vec<DaemonNotice> {
    use serde_json::Value;
    match probe.get("status").and_then(Value::as_str) {
        Some("online" | "warning") => {}
        Some("unconfigured") => {
            return vec![DaemonNotice::new(
                "lnd_unconfigured",
                "El panel no tiene acceso de lectura a LND: no puede comprobar los canales ni la sincronización.",
            )];
        }
        _ => {
            return vec![DaemonNotice::new(
                "lnd_unreadable",
                "El panel no pudo leer LND: no puede comprobar los canales ni la sincronización. Revisa la conexión, el certificado y los permisos.",
            )];
        }
    }
    let mut notices = Vec::new();
    match probe.get("num_active_channels").and_then(Value::as_u64) {
        Some(0) => notices.push(DaemonNotice::new("lnd_no_channels", "LND reporta 0 canales activos. Mostro requiere canales Lightning abiertos para crear y liquidar hold invoices.")),
        Some(_) => {}
        None => notices.push(DaemonNotice::new(
            "lnd_channels_unknown",
            "LND no informó de cuántos canales activos tiene.",
        )),
    }
    if probe.get("synced_to_chain").and_then(Value::as_bool) != Some(true) {
        notices.push(DaemonNotice::new(
            "lnd_not_synced",
            "LND no está sincronizado con la cadena de bloques.",
        ));
    }
    notices
}

/// Explicit activation requested by the operator: writes the settings and
/// (re)starts the daemon even when the settings did not change.
pub fn activate(root: &Path, lnd_grpc_origin: &str) -> Result<ActivationResult, &'static str> {
    apply_settings(root, lnd_grpc_origin, true)
}

/// Brings the active settings in line with the saved draft. The daemon is
/// restarted only when the rendered `settings.toml` actually changes.
pub fn sync_active_settings(
    root: &Path,
    lnd_grpc_origin: &str,
) -> Result<ActivationResult, &'static str> {
    apply_settings(root, lnd_grpc_origin, false)
}

fn apply_settings(
    root: &Path,
    lnd_grpc_origin: &str,
    restart_when_unchanged: bool,
) -> Result<ActivationResult, &'static str> {
    let origin = Url::parse(lnd_grpc_origin).map_err(|_| "Origen LND inválido")?;
    if origin.port().is_none() {
        return Err("El origen LND gRPC debe incluir un puerto explícito");
    }
    let root_metadata = fs::symlink_metadata(root).map_err(|_| "CONFIG_DIR no disponible")?;
    if !root_metadata.is_dir() || root_metadata.permissions().mode() & 0o077 != 0 {
        return Err("CONFIG_DIR debe ser un directorio privado (0700)");
    }
    if identity::read_private(root)?.is_none() {
        return Err("No se puede activar el demonio sin una identidad privada importada");
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

    let settings = render_settings(config, lnd_grpc_origin, CERT_PATH, MACAROON_PATH)
        .map_err(|_| "No se pudo generar settings.toml")?;

    let active_dir = root.join("active");
    ensure_private_dir(&active_dir)?;

    let settings_file = active_dir.join("settings.toml");
    let status_file = active_dir.join("status.json");
    let now_unix = unix_now();

    // A draft that renders the same settings (a payment label, the contact
    // link) must not restart a daemon that may have trades in flight. Nor
    // must a file that only differs in how an earlier version wrote it.
    let on_disk = fs::read(&settings_file).ok();
    let identical = on_disk.as_deref() == Some(settings.as_bytes());
    let settings_changed = !on_disk
        .as_deref()
        .is_some_and(|current| identical || same_settings(current, &settings));
    // An explicit activation restarts the daemon anyway, so it also brings
    // an equivalent file up to date.
    let rewrite = settings_changed || (restart_when_unchanged && !identical);

    // The hash is the one of the file the daemon reads.
    let active_bytes = match &on_disk {
        Some(current) if !rewrite => current.as_slice(),
        _ => settings.as_bytes(),
    };
    let mut hasher = Sha256::new();
    hasher.update(active_bytes);
    let settings_sha256 = format!("{:x}", hasher.finalize());

    let status = ActiveStatus {
        revision: document.revision,
        settings_sha256: settings_sha256.clone(),
        lnd_grpc_origin: lnd_grpc_origin.to_string(),
        activated_at_unix: now_unix,
    };

    if rewrite {
        write_private(&settings_file, settings.as_bytes())?;
    }
    // Ask for the restart as soon as the new settings are on disk. If a later
    // step failed first, the daemon would keep running the old settings while
    // the next save found the file already up to date and never restarted it.
    //
    // The daemon publishes its own kind 0, 10002 and 38385 events when it
    // starts. The panel never signs protocol events with the node key: an
    // info event from here would advertise a node whether or not mostrod came up.
    // The one event the panel does sign, the community card (`card_publication`),
    // is not part of the protocol and says nothing about a daemon running.
    if settings_changed || restart_when_unchanged {
        notify_standby(root);
    }

    let status_bytes = serde_json::to_vec_pretty(&status)
        .map_err(|_| "No se pudo serializar el estado de activación")?;
    write_private(&status_file, &status_bytes)?;

    File::open(&active_dir)
        .and_then(|file| file.sync_all())
        .map_err(|_| "No se pudo sincronizar el directorio activo")?;

    Ok(ActivationResult {
        revision: document.revision,
        settings_path: settings_file,
        settings_sha256,
        activated_at_unix: now_unix,
        settings_changed,
    })
}

pub fn deactivate(root: &Path) -> Result<(), &'static str> {
    let active_dir = root.join("active");
    let settings_file = active_dir.join("settings.toml");
    let status_file = active_dir.join("status.json");

    if settings_file.exists() {
        fs::remove_file(&settings_file).map_err(|_| "No se pudo eliminar settings.toml activo")?;
    }
    if status_file.exists() {
        fs::remove_file(&status_file).map_err(|_| "No se pudo eliminar status.json activo")?;
    }
    let _ = fs::remove_file(root.join(".standby_wake"));
    let _ = fs::remove_file(active_dir.join(".standby_wake"));
    let _ = fs::remove_file(active_dir.join("mostro.pid"));
    let _ = fs::remove_file(active_dir.join("mostro.heartbeat"));
    let _ = fs::remove_file(active_dir.join("mostro.last_exit"));

    notify_standby(root);

    for pid in mostrod_pids(&active_dir) {
        let _ = std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status();
    }

    Ok(())
}

/// Asks the supervisor in the daemon container to (re)start mostrod. The
/// request is a file inside CONFIG_DIR and nowhere else: in Umbrel that is the
/// volume both containers share, and a test or a CLI run with another
/// CONFIG_DIR can never reach a production daemon.
fn notify_standby(root: &Path) {
    let _ = File::create(root.join(".standby_wake"));
    let _ = File::create(root.join("active").join(".standby_wake"));
}

fn uses_settings_directory(cmdline: &[u8], settings_dir: &Path) -> bool {
    let args: Vec<&[u8]> = cmdline.split(|byte| *byte == 0).collect();
    args.windows(2)
        .any(|pair| pair[0] == b"-d" && pair[1] == settings_dir.as_os_str().as_bytes())
}

/// The daemon is visible in this container, or the entrypoint that supervises
/// it in the daemon container touched its heartbeat within the last 30 s. A
/// pid file alone proves nothing: it survives a hard kill.
fn is_mostrod_running(settings_dir: &Path) -> bool {
    if !mostrod_pids(settings_dir).is_empty() {
        return true;
    }
    fs::metadata(settings_dir.join("mostro.heartbeat"))
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|elapsed| elapsed.as_secs() < 30)
}

fn mostrod_pids(settings_dir: &Path) -> Vec<u32> {
    let mut pids = Vec::new();
    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            if let Ok(file_name) = entry.file_name().into_string()
                && file_name.chars().all(|c| c.is_ascii_digit())
            {
                let comm_path = entry.path().join("comm");
                if let Ok(comm) = fs::read_to_string(&comm_path)
                    && comm.trim() == "mostrod"
                    && fs::read(entry.path().join("cmdline"))
                        .is_ok_and(|args| uses_settings_directory(&args, settings_dir))
                    && let Ok(pid) = file_name.parse::<u32>()
                    && pid > 1
                {
                    pids.push(pid);
                }
            }
        }
    }
    pids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_detection_requires_this_instances_settings_directory() {
        let dir = Path::new("/data/config/active");
        assert!(uses_settings_directory(
            b"/usr/local/bin/mostrod\0-d\0/data/config/active\0",
            dir
        ));
        for cmdline in [
            b"mostrod\0-d\0/another/instance\0".as_slice(),
            b"mostrod\0-d\0/data/config/active-old\0",
            b"mostrod\0",
            b"mostrod\0-d\0",
            b"",
        ] {
            assert!(!uses_settings_directory(cmdline, dir));
        }
    }
}
