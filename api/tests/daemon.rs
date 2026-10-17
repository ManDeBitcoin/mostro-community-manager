use mostro_community_api::{
    adapters::Integrations,
    config::Configuration,
    daemon::{
        DaemonReport, DaemonState, MOSTRO_VERSION, NOTICE_CODES, activate, deactivate, report,
        report_with_node_info, sync_active_settings, sync_with_recorded_origin,
    },
    identity,
    orders::NodeInfo,
    store::Store,
};
use nostr::{Keys, SecretKey, ToBech32};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// A private CONFIG_DIR with a saved draft and an imported identity.
fn configured_root(temp: &tempfile::TempDir, seed: u8) -> (PathBuf, Store) {
    let root = temp.path().join("config");
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let mut store = Store::open(root.clone()).unwrap();
    store.save(config()).unwrap();
    let keys = Keys::new(SecretKey::from_slice(&[seed; 32]).unwrap());
    identity::import(
        &root,
        &keys.secret_key().to_bech32().unwrap(),
        &keys.public_key().to_bech32().unwrap(),
    )
    .unwrap();
    (root, store)
}

fn announced(version: &str, created_at: u64) -> NodeInfo {
    NodeInfo {
        event_id: "a".repeat(64),
        created_at,
        name: Some("Comunidad de prueba".into()),
        mostro_version: Some(version.into()),
        protocol_version: Some("2".into()),
        tags: BTreeMap::from([("mostro_version".to_string(), version.to_string())]),
    }
}

/// What the container entrypoint leaves while mostrod is alive.
fn mark_alive(active_dir: &Path) {
    fs::write(active_dir.join("mostro.pid"), "4242\n").unwrap();
    fs::write(active_dir.join("mostro.heartbeat"), "").unwrap();
}

/// Puts the start of the current mostrod process `secs` in the past. The
/// entrypoint writes the pid file once, when it starts the daemon, so the age
/// of that file is how long the process has been running.
fn started_secs_ago(active_dir: &Path, secs: u64) {
    fs::File::options()
        .write(true)
        .open(active_dir.join("mostro.pid"))
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(secs))
        .unwrap();
}

fn codes(rep: &DaemonReport) -> Vec<&str> {
    rep.notices
        .iter()
        .map(|notice| notice.code.as_str())
        .collect()
}

fn config() -> Configuration {
    serde_json::from_str(include_str!("fixtures/community.json")).unwrap()
}

#[tokio::test]
async fn reports_unconfigured_daemon_state() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("config");
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();

    let rep = report(&root, &Integrations::from_env()).await;
    assert_eq!(rep.state, DaemonState::Unconfigured);
    assert!(!rep.identity_present);
    assert!(rep.npub.is_none());
    assert!(rep.draft_revision.is_none());
    assert!(rep.active_revision.is_none());
    assert!(!rep.can_activate);
    assert!(!rep.warnings.is_empty());
}

#[tokio::test]
async fn lifecycle_standby_to_activated_to_deactivated() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("config");
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();

    let mut store = Store::open(root.clone()).unwrap();
    store.save(config()).unwrap();

    let keys = Keys::new(SecretKey::from_slice(&[7; 32]).unwrap());
    let secret = keys.secret_key().to_bech32().unwrap();
    let public = keys.public_key().to_bech32().unwrap();
    identity::import(&root, &secret, &public).unwrap();

    let rep = report(&root, &Integrations::from_env()).await;
    assert_eq!(rep.state, DaemonState::ConfiguredStandby);
    assert!(rep.identity_present);
    assert_eq!(rep.npub.as_deref(), Some(public.as_str()));
    assert_eq!(rep.draft_revision, Some(1));
    assert!(rep.active_revision.is_none());
    assert!(rep.can_activate);

    let activated = activate(&root, "https://127.0.0.1:10009").unwrap();
    assert_eq!(activated.revision, 1);
    assert_eq!(activated.settings_sha256.len(), 64);
    assert!(activated.settings_path.is_file());

    let active_dir = root.join("active");
    assert_eq!(
        fs::metadata(&active_dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let settings_path = active_dir.join("settings.toml");
    assert_eq!(
        fs::metadata(&settings_path).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let settings_str = fs::read_to_string(&settings_path).unwrap();
    assert!(!settings_str.contains("nsec1"));
    assert!(settings_str.contains("https://127.0.0.1:10009"));
    assert!(settings_str.contains("fee = 0.006"));

    let rep_active = report(&root, &Integrations::from_env()).await;
    assert!(matches!(
        rep_active.state,
        DaemonState::ActiveReady | DaemonState::ActiveRunning
    ));
    assert_eq!(rep_active.active_revision, Some(1));
    assert_eq!(
        rep_active.active_settings_hash.as_deref(),
        Some(activated.settings_sha256.as_str())
    );
    if rep_active.state == DaemonState::ActiveReady {
        assert!(
            rep_active
                .warnings
                .iter()
                .any(|warning| warning.contains("ejecución del daemon sin verificar"))
        );
    }

    let mut updated = config();
    updated.market.max_trade_sats += 1;
    store.save(updated).unwrap();
    let changed = report(&root, &Integrations::default()).await;
    assert_eq!(changed.draft_revision, Some(2));
    assert_eq!(changed.active_revision, Some(1));
    assert!(
        changed
            .warnings
            .iter()
            .any(|warning| warning.contains("el nodo aún no tiene aplicadas"))
    );

    deactivate(&root).unwrap();
    assert!(!settings_path.exists());
    assert!(!active_dir.join("status.json").exists());

    let rep_after = report(&root, &Integrations::from_env()).await;
    assert_eq!(rep_after.state, DaemonState::ConfiguredStandby);
    assert!(rep_after.active_revision.is_none());
    assert!(rep_after.active_settings_hash.is_none());
}

#[tokio::test]
async fn activation_fails_without_identity_or_invalid_origin() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("config");
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();

    let mut store = Store::open(root.clone()).unwrap();
    store.save(config()).unwrap();

    let err = activate(&root, "https://127.0.0.1:10009").unwrap_err();
    assert_eq!(
        err,
        "No se puede activar el demonio sin una identidad privada importada"
    );

    let keys = Keys::new(SecretKey::from_slice(&[8; 32]).unwrap());
    let secret = keys.secret_key().to_bech32().unwrap();
    let public = keys.public_key().to_bech32().unwrap();
    identity::import(&root, &secret, &public).unwrap();

    assert!(activate(&root, "http://127.0.0.1:10009").is_err());
    assert!(activate(&root, "https://127.0.0.1").is_err());
    assert!(activate(&root, "not-a-url").is_err());
}

/// What the API does when it starts: settings written by an earlier version
/// (here, without the `[health]` section the v0.20.0 template adds) are
/// rewritten once, with the LND origin recorded at activation, and the daemon
/// is woken; the next start finds them current and leaves the daemon alone.
#[test]
fn startup_applies_a_new_rendering_once() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _store) = configured_root(&temp, 32);
    let origin = "https://127.0.0.1:10009";
    let wake = root.join(".standby_wake");
    let settings = root.join("active").join("settings.toml");

    // Nothing activated yet: nothing to apply.
    assert!(sync_with_recorded_origin(&root).unwrap().is_none());

    activate(&root, origin).unwrap();
    fs::remove_file(&wake).unwrap();
    let current = fs::read_to_string(&settings).unwrap();
    let mut older: toml::Value = toml::from_str(&current).unwrap();
    older.as_table_mut().unwrap().remove("health").unwrap();
    fs::write(&settings, toml::to_string_pretty(&older).unwrap()).unwrap();

    let applied = sync_with_recorded_origin(&root).unwrap().unwrap();
    assert!(applied.settings_changed);
    assert!(wake.exists());
    assert_eq!(fs::read_to_string(&settings).unwrap(), current);
    fs::remove_file(&wake).unwrap();

    let again = sync_with_recorded_origin(&root).unwrap().unwrap();
    assert!(!again.settings_changed);
    assert!(
        !wake.exists(),
        "a current settings.toml must not restart mostrod"
    );

    // Without the activation record the origin is unknown: nothing is applied.
    fs::remove_file(root.join("active").join("status.json")).unwrap();
    assert!(sync_with_recorded_origin(&root).is_err());
}

#[tokio::test]
async fn saving_an_equivalent_draft_does_not_restart_the_daemon() {
    let temp = tempfile::tempdir().unwrap();
    let (root, mut store) = configured_root(&temp, 31);
    let origin = "https://127.0.0.1:10009";
    let wake = root.join(".standby_wake");
    let settings = root.join("active").join("settings.toml");

    // The first activation writes the settings and wakes the daemon.
    let first = activate(&root, origin).unwrap();
    assert!(first.settings_changed);
    assert!(wake.exists());
    let rendered = fs::read(&settings).unwrap();
    fs::remove_file(&wake).unwrap();

    // A change that never reaches settings.toml (a payment method label)
    // leaves the daemon alone; only the active revision moves on.
    let mut relabelled = config();
    relabelled.payment_methods[0].label = "Transferencia SEPA".into();
    store.save(relabelled).unwrap();
    let synced = sync_active_settings(&root, origin).unwrap();
    assert!(!synced.settings_changed);
    assert_eq!(synced.revision, 2);
    assert!(
        !wake.exists(),
        "an unchanged settings.toml must not restart mostrod"
    );
    assert_eq!(fs::read(&settings).unwrap(), rendered);
    let rep = report(&root, &Integrations::default()).await;
    assert_eq!(rep.active_revision, Some(2));
    assert!(
        !rep.warnings
            .iter()
            .any(|w| w.contains("el nodo aún no tiene aplicadas"))
    );

    // A change that does reach settings.toml restarts it.
    let mut repriced = config();
    repriced.market.fee_bps = 80;
    store.save(repriced).unwrap();
    let synced = sync_active_settings(&root, origin).unwrap();
    assert!(synced.settings_changed);
    assert!(wake.exists());
    assert!(
        fs::read_to_string(&settings)
            .unwrap()
            .contains("fee = 0.008")
    );
    fs::remove_file(&wake).unwrap();

    // An explicit activation always restarts, even with nothing to change.
    let again = activate(&root, origin).unwrap();
    assert!(!again.settings_changed);
    assert!(wake.exists());

    // Nothing is left behind by the atomic write.
    let leftovers: Vec<_> = fs::read_dir(root.join("active"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    assert_eq!(
        fs::metadata(&settings).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[tokio::test]
async fn reports_the_version_the_daemon_announces_when_it_is_recent() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _store) = configured_root(&temp, 32);
    activate(&root, "https://127.0.0.1:10009").unwrap();
    let integrations = Integrations::default();

    // Nothing on the relays: only the packaged binary (or the pin) is known.
    let rep = report(&root, &integrations).await;
    assert_ne!(rep.version_source, "announced");
    assert_eq!(rep.mostro_version, rep.packaged_version);
    assert!(rep.announced.is_none());
    assert!(!rep.announced_fresh);
    assert_eq!(rep.announced_age_secs, None);
    if rep.version_source == "pinned" {
        assert_eq!(rep.packaged_version, MOSTRO_VERSION);
    }

    // A recent info event from the daemon is the authoritative version.
    let rep =
        report_with_node_info(&root, &integrations, Some(announced("0.19.2", now() - 30))).await;
    assert_eq!(rep.mostro_version, "0.19.2");
    assert_eq!(rep.version_source, "announced");
    assert!(rep.announced_fresh);
    assert!(
        rep.announced_age_secs
            .is_some_and(|age| (30..40).contains(&age))
    );
    // This panel does not see a daemon running, yet one is announcing the
    // identity: a second instance must not be started.
    assert_eq!(rep.state, DaemonState::ActiveReady);
    assert!(
        rep.warnings
            .iter()
            .any(|w| w.contains("otra instancia de Mostro usa esta misma clave"))
    );

    // An old announcement proves nothing about the present.
    let rep = report_with_node_info(
        &root,
        &integrations,
        Some(announced("0.19.0", now() - 3600)),
    )
    .await;
    assert_ne!(rep.version_source, "announced");
    assert_eq!(rep.mostro_version, rep.packaged_version);
    assert!(!rep.announced_fresh);
    assert_eq!(
        rep.announced
            .as_ref()
            .and_then(|a| a.mostro_version.as_deref()),
        Some("0.19.0")
    );
    assert!(
        !rep.warnings
            .iter()
            .any(|w| w.contains("otra instancia de Mostro usa esta misma clave"))
    );
}

#[tokio::test]
async fn running_state_needs_a_live_heartbeat_and_a_stable_process() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _store) = configured_root(&temp, 33);
    activate(&root, "https://127.0.0.1:10009").unwrap();
    let active = root.join("active");
    let integrations = Integrations::default();

    // A pid file left by a hard kill does not mean the daemon is running.
    fs::write(active.join("mostro.pid"), "4242\n").unwrap();
    let rep = report(&root, &integrations).await;
    assert_eq!(rep.state, DaemonState::ActiveReady);

    // The entrypoint's heartbeat does.
    mark_alive(&active);
    started_secs_ago(&active, 600);
    let rep =
        report_with_node_info(&root, &integrations, Some(announced("0.19.2", now() - 5))).await;
    assert_eq!(rep.state, DaemonState::ActiveRunning);
    assert!(!rep.warnings.iter().any(|w| w.contains("sin verificar")));
    assert!(
        !rep.warnings
            .iter()
            .any(|w| w.contains("otra instancia de Mostro usa esta misma clave"))
    );

    // A daemon announcing another version than the packaged one is flagged.
    let rep =
        report_with_node_info(&root, &integrations, Some(announced("0.17.5", now() - 5))).await;
    assert_eq!(rep.mostro_version, "0.17.5");
    assert!(
        rep.warnings
            .iter()
            .any(|w| w.contains("anuncia la versión 0.17.5"))
    );

    // The daemon died two seconds after starting and was just respawned: the
    // heartbeat is fresh, but it is not a running node.
    mark_alive(&active);
    fs::write(
        active.join("mostro.last_exit"),
        format!("{} 1 2\n", now() - 3),
    )
    .unwrap();
    let rep = report(&root, &integrations).await;
    assert_eq!(rep.state, DaemonState::ActiveReady);
    let exit = rep.last_exit.expect("last exit");
    assert_eq!((exit.code, exit.uptime_secs), (1, 2));
    assert!(
        rep.warnings
            .iter()
            .any(|w| w.contains("terminó con código 1"))
    );

    // An old exit followed by a healthy run is history, not a crash loop.
    fs::write(
        active.join("mostro.last_exit"),
        format!("{} 1 2\n", now() - 900),
    )
    .unwrap();
    let rep = report(&root, &integrations).await;
    assert_eq!(rep.state, DaemonState::ActiveRunning);
    assert!(rep.last_exit.is_some());

    // Deactivation clears the runtime markers.
    deactivate(&root).unwrap();
    assert!(!active.join("mostro.last_exit").exists());
    assert!(!active.join("mostro.heartbeat").exists());
}

#[tokio::test]
async fn warns_when_payouts_cannot_pay_any_routing_fee() {
    let temp = tempfile::tempdir().unwrap();
    let (root, mut store) = configured_root(&temp, 34);
    let rep = report(&root, &Integrations::default()).await;
    assert!(!rep.warnings.iter().any(|w| w.contains("enrutamiento")));

    let mut free_routes_only = config();
    free_routes_only.market.max_routing_fee_bps = 0;
    store.save(free_routes_only).unwrap();
    let rep = report(&root, &Integrations::default()).await;
    assert!(rep.warnings.iter().any(|w| w.contains("enrutamiento es 0")));
}

/// Verified on regtest against mostrod v0.19.2: with `pow_first_contact`
/// above `pow`, a client that does not mine it gets no answer at all.
#[tokio::test]
async fn warns_when_first_contact_pow_can_lock_clients_out() {
    let temp = tempfile::tempdir().unwrap();
    let (root, mut store) = configured_root(&temp, 35);
    let warned = |rep: &mostro_community_api::daemon::DaemonReport| {
        rep.warnings.iter().any(|w| w.contains("pow_first_contact"))
    };

    for (pow, first_contact, expected) in [(0, 0, false), (8, 8, false), (0, 8, true)] {
        let mut draft = config();
        draft.safety.pow = pow;
        draft.safety.pow_first_contact = first_contact;
        store.save(draft).unwrap();
        let rep = report(&root, &Integrations::default()).await;
        assert_eq!(
            warned(&rep),
            expected,
            "pow {pow}, first contact {first_contact}"
        );
    }
}

/// A probe that could not read LND knows nothing about its channels.
#[test]
fn lnd_notices_do_not_invent_what_the_probe_did_not_read() {
    use mostro_community_api::daemon::lnd_notices;
    use serde_json::json;
    let codes = |probe: serde_json::Value| -> Vec<String> {
        lnd_notices(&probe)
            .into_iter()
            .map(|notice| notice.code)
            .collect()
    };

    assert_eq!(
        codes(json!({"status": "unconfigured"})),
        ["lnd_unconfigured"]
    );
    assert_eq!(codes(json!({"status": "offline"})), ["lnd_unreadable"]);
    assert_eq!(codes(json!({})), ["lnd_unreadable"]);
    for probe in [
        json!({"status": "unconfigured"}),
        json!({"status": "offline"}),
        json!({}),
    ] {
        let text = lnd_notices(&probe)[0].text.clone();
        assert!(!text.contains("0 canales"), "{text}");
        assert!(text.contains("no puede comprobar"), "{text}");
    }

    let healthy = json!({"status": "online", "synced_to_chain": true, "num_active_channels": 2});
    assert!(lnd_notices(&healthy).is_empty());

    let no_channels =
        json!({"status": "online", "synced_to_chain": true, "num_active_channels": 0});
    assert_eq!(codes(no_channels), ["lnd_no_channels"]);

    let syncing =
        json!({"status": "warning", "synced_to_chain": false, "num_active_channels": null});
    assert_eq!(codes(syncing), ["lnd_channels_unknown", "lnd_not_synced"]);
}

#[tokio::test]
async fn report_without_lnd_access_says_so_instead_of_zero_channels() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _store) = configured_root(&temp, 36);
    let rep = report(&root, &Integrations::default()).await;
    assert!(
        rep.notices
            .iter()
            .any(|notice| notice.code == "lnd_unconfigured")
    );
    assert!(!rep.warnings.iter().any(|w| w.contains("0 canales")));
    // Unknown is not zero, in the fields a script would read either.
    assert_eq!(rep.lnd_channel_count, None);
    assert_eq!(rep.lnd_synced, None);
}

/// Every notice carries a code the panel knows, and `warnings` is the same
/// list as plain sentences.
#[tokio::test]
async fn notices_have_known_codes_and_mirror_the_warnings() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("config");
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let empty = report(&root, &Integrations::default()).await;

    let temp = tempfile::tempdir().unwrap();
    let (configured, mut store) = configured_root(&temp, 37);
    let mut draft = config();
    draft.market.max_routing_fee_bps = 0;
    draft.safety.pow = 0;
    draft.safety.pow_first_contact = 8;
    store.save(draft).unwrap();
    let standby = report(&configured, &Integrations::default()).await;

    for rep in [&empty, &standby] {
        assert!(!rep.notices.is_empty());
        for notice in &rep.notices {
            assert!(
                NOTICE_CODES.contains(&notice.code.as_str()),
                "unknown code {}",
                notice.code
            );
        }
        let texts: Vec<&str> = rep.notices.iter().map(|n| n.text.as_str()).collect();
        assert_eq!(
            texts,
            rep.warnings.iter().map(String::as_str).collect::<Vec<_>>()
        );
    }
    assert_eq!(
        codes(&empty),
        ["identity_missing", "rules_missing", "lnd_unconfigured"]
    );
    assert_eq!(
        codes(&standby),
        [
            "lnd_unconfigured",
            "routing_fee_zero",
            "pow_first_contact_above_base"
        ]
    );
}

/// Right after an update or a restart the relays still hold what the previous
/// process announced. That is neither the version of the process running now
/// nor proof that it is serving clients.
#[tokio::test]
async fn an_announcement_older_than_the_running_process_is_not_its_own() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _store) = configured_root(&temp, 38);
    activate(&root, "https://127.0.0.1:10009").unwrap();
    let active = root.join("active");
    let integrations = Integrations::default();
    let seen = |version: &str, age: u64| Some(announced(version, now() - age));

    // The daemon started 20 s ago; the relays hold an announcement of 3 min.
    mark_alive(&active);
    started_secs_ago(&active, 20);
    let rep = report_with_node_info(&root, &integrations, seen("0.17.5", 180)).await;
    assert_eq!(rep.state, DaemonState::ActiveRunning);
    assert!(
        rep.running_for_secs
            .is_some_and(|secs| (20..30).contains(&secs))
    );
    assert!(!rep.announced_fresh, "the market is not open yet");
    assert!(rep.announced_before_start);
    assert!(
        !codes(&rep).contains(&"version_mismatch"),
        "{:?}",
        codes(&rep)
    );
    assert!(!codes(&rep).contains(&"announcement_stale"));
    assert_ne!(rep.version_source, "announced");
    assert_eq!(rep.mostro_version, rep.packaged_version);
    // What the relays hold is still reported as it was read.
    assert_eq!(
        rep.announced
            .as_ref()
            .and_then(|a| a.mostro_version.as_deref()),
        Some("0.17.5")
    );
    assert!(
        rep.announced_age_secs
            .is_some_and(|age| (180..190).contains(&age))
    );

    // The same announcement from a daemon that was already running is its own.
    started_secs_ago(&active, 600);
    let rep = report_with_node_info(&root, &integrations, seen("0.17.5", 180)).await;
    assert!(rep.announced_fresh);
    assert!(!rep.announced_before_start);
    assert_eq!(rep.mostro_version, "0.17.5");
    assert!(codes(&rep).contains(&"version_mismatch"));

    // Both ages are whole seconds read at different moments: a few seconds of
    // difference do not turn the first announcement of a daemon into an old one.
    started_secs_ago(&active, 100);
    let rep = report_with_node_info(&root, &integrations, seen("0.19.2", 103)).await;
    assert!(rep.announced_fresh);
    let rep = report_with_node_info(&root, &integrations, seen("0.19.2", 112)).await;
    assert!(!rep.announced_fresh);
    assert!(rep.announced_before_start);

    // An announcement that is simply old is not "from before this start".
    let rep = report_with_node_info(&root, &integrations, seen("0.19.2", 3600)).await;
    assert!(!rep.announced_fresh);
    assert!(!rep.announced_before_start);

    // Without a pid file the age of the process is unknown, and what is not
    // known cannot rule an announcement out.
    fs::remove_file(active.join("mostro.pid")).unwrap();
    let rep = report_with_node_info(&root, &integrations, seen("0.17.5", 180)).await;
    assert_eq!(rep.state, DaemonState::ActiveRunning);
    assert_eq!(rep.running_for_secs, None);
    assert!(rep.announced_fresh);
    assert!(!rep.announced_before_start);
    assert!(codes(&rep).contains(&"version_mismatch"));
}

/// Up to v1.0.11 the profile fields were written as typed, also when empty.
/// Such a file says the same to mostrod as the one rendered today: the first
/// save after updating must not restart a daemon over it.
#[tokio::test]
async fn a_settings_file_written_by_an_earlier_version_does_not_restart_the_daemon() {
    let temp = tempfile::tempdir().unwrap();
    let (root, mut store) = configured_root(&temp, 39);
    let origin = "https://127.0.0.1:10009";
    let wake = root.join(".standby_wake");
    let settings = root.join("active").join("settings.toml");

    let mut draft = config();
    draft.community.name = "Comunidad de prueba ".into();
    draft.community.about = String::new();
    assert!(draft.community.website.is_empty());
    store.save(draft.clone()).unwrap();
    activate(&root, origin).unwrap();
    let rendered = fs::read_to_string(&settings).unwrap();
    let mut earlier: toml::Value = toml::from_str(&rendered).unwrap();
    assert!(earlier["mostro"].get("about").is_none());
    assert!(earlier["mostro"].get("website").is_none());

    // The same settings as v1.0.11 wrote them.
    let mostro = earlier["mostro"].as_table_mut().unwrap();
    mostro.insert("name".into(), draft.community.name.as_str().into());
    mostro.insert("about".into(), "".into());
    mostro.insert("website".into(), "".into());
    let earlier = toml::to_string_pretty(&earlier).unwrap();
    assert_ne!(earlier, rendered);
    fs::write(&settings, &earlier).unwrap();
    fs::remove_file(&wake).unwrap();

    // The first save after the update, of something that never reaches the file.
    let mut relabelled = draft.clone();
    relabelled.payment_methods[0].label = "Transferencia SEPA".into();
    store.save(relabelled).unwrap();
    let synced = sync_active_settings(&root, origin).unwrap();
    assert!(!synced.settings_changed);
    assert!(
        !wake.exists(),
        "an equivalent settings.toml must not restart mostrod"
    );
    assert_eq!(fs::read_to_string(&settings).unwrap(), earlier);
    // The recorded hash is the one of the file the daemon reads.
    let rep = report(&root, &Integrations::default()).await;
    assert_eq!(rep.active_revision, Some(synced.revision));
    assert_eq!(
        rep.active_settings_hash.as_deref(),
        Some(synced.settings_sha256.as_str())
    );

    // An explicit activation restarts anyway, and brings the file up to date.
    let again = activate(&root, origin).unwrap();
    assert!(!again.settings_changed);
    assert!(wake.exists());
    assert_eq!(fs::read_to_string(&settings).unwrap(), rendered);
    fs::remove_file(&wake).unwrap();

    // A profile field that really changed still restarts the daemon.
    fs::write(&settings, &earlier).unwrap();
    let mut described = draft.clone();
    described.community.about = "Mercado local".into();
    store.save(described).unwrap();
    let synced = sync_active_settings(&root, origin).unwrap();
    assert!(synced.settings_changed);
    assert!(wake.exists());
    assert!(
        fs::read_to_string(&settings)
            .unwrap()
            .contains("about = \"Mercado local\"")
    );
    fs::remove_file(&wake).unwrap();

    // So does a file that cannot be read as settings at all.
    fs::write(&settings, "esto no es = = TOML").unwrap();
    let synced = sync_active_settings(&root, origin).unwrap();
    assert!(synced.settings_changed);
    assert!(wake.exists());
}

/// One exit after hours of running is not a crash loop, and the supervisor has
/// the daemon back within seconds: without a notice the operator never knows.
#[tokio::test]
async fn an_unexpected_exit_is_a_notice_while_it_is_recent() {
    let temp = tempfile::tempdir().unwrap();
    let (root, _store) = configured_root(&temp, 40);
    activate(&root, "https://127.0.0.1:10009").unwrap();
    let active = root.join("active");
    let integrations = Integrations::default();
    let exited = |ago: u64, code: i32, uptime: u64| {
        fs::write(
            active.join("mostro.last_exit"),
            format!("{} {code} {uptime}\n", now() - ago),
        )
        .unwrap();
    };
    assert!(NOTICE_CODES.contains(&"daemon_unexpected_exit"));

    // It ran for two hours, ended ten minutes ago and was started again.
    mark_alive(&active);
    started_secs_ago(&active, 590);
    exited(600, 101, 7200);
    let rep = report(&root, &integrations).await;
    assert_eq!(rep.state, DaemonState::ActiveRunning);
    assert!(!codes(&rep).contains(&"daemon_crash_loop"));
    let notice = rep
        .notices
        .iter()
        .find(|notice| notice.code == "daemon_unexpected_exit")
        .expect("a recent unexpected exit is a notice");
    for expected in ["hace 10 min", "código 101", "tras 2 h", "volvió a arrancar"] {
        assert!(notice.text.contains(expected), "{}", notice.text);
    }
    assert!(rep.warnings.contains(&notice.text));

    // A code above 128 is how the supervisor's shell reports a signal, such as
    // the kill of a process that ran out of memory: the notice says which.
    exited(600, 137, 7200);
    let rep = report(&root, &integrations).await;
    assert!(
        rep.warnings
            .iter()
            .any(|w| w.contains("con código 137 (señal 9) y tras 2 h")),
        "{:?}",
        rep.warnings
    );

    // While the supervisor waits before its next attempt, it says that instead.
    fs::remove_file(active.join("mostro.heartbeat")).unwrap();
    fs::remove_file(active.join("mostro.pid")).unwrap();
    let rep = report(&root, &integrations).await;
    assert_eq!(rep.state, DaemonState::ActiveReady);
    let notice = rep
        .notices
        .iter()
        .find(|notice| notice.code == "daemon_unexpected_exit")
        .expect("still a notice while the daemon is down");
    assert!(
        notice.text.contains("lo vuelve a intentar"),
        "{}",
        notice.text
    );

    // A crash loop has its own notice: the same exit is not told twice.
    mark_alive(&active);
    exited(3, 1, 2);
    let rep = report(&root, &integrations).await;
    assert!(codes(&rep).contains(&"daemon_crash_loop"));
    assert!(!codes(&rep).contains(&"daemon_unexpected_exit"));

    // After an hour it is history, kept in the report and out of the notices.
    exited(3700, 101, 7200);
    let rep = report(&root, &integrations).await;
    assert_eq!(rep.state, DaemonState::ActiveRunning);
    assert!(rep.last_exit.is_some());
    assert!(!codes(&rep).contains(&"daemon_unexpected_exit"));

    // A record left behind once Mostro is deactivated is nobody's news.
    deactivate(&root).unwrap();
    fs::write(
        active.join("mostro.last_exit"),
        format!("{} 143 60\n", now() - 5),
    )
    .unwrap();
    let rep = report(&root, &integrations).await;
    assert_eq!(rep.state, DaemonState::ConfiguredStandby);
    assert!(!codes(&rep).contains(&"daemon_unexpected_exit"));
}
