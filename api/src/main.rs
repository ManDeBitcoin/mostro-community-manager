use mostro_community_api::{AppState, adapters::Integrations, router, store::Store};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        if args == ["import-identity"] {
            return mostro_community_api::identity::import_interactive()
                .map_err(|message| message.into());
        }
        if args == ["export-backup"] {
            let root = PathBuf::from(
                std::env::var("CONFIG_DIR").unwrap_or_else(|_| "./var/config".into()),
            );
            return mostro_community_api::backup::export_interactive(&root)
                .map_err(|message| message.into());
        }
        if args.len() == 2 && args[0] == "verify-backup" {
            return mostro_community_api::backup::verify_interactive(&PathBuf::from(&args[1]))
                .map_err(|message| message.into());
        }
        if args.len() == 3 && args[0] == "restore-backup" {
            return mostro_community_api::backup::restore_interactive(
                &PathBuf::from(&args[1]),
                &PathBuf::from(&args[2]),
            )
            .map_err(|message| message.into());
        }
        // The settings.toml the daemon would get from saved rules (a
        // community.json, or a bare configuration) or from a preset, for
        // checking it against the real mostrod (scripts/daemon-contract-smoke.sh).
        if (args.len() == 2 || args.len() == 3) && args[0] == "render-settings" {
            let config = match args[1].strip_prefix("preset:") {
                Some(id) => {
                    mostro_community_api::config::get_regional_presets()
                        .into_iter()
                        .find(|preset| preset.id == id)
                        .ok_or("Plantilla de comisiones desconocida")?
                        .config
                }
                None => {
                    let bytes = std::fs::read(&args[1])?;
                    match serde_json::from_slice::<mostro_community_api::config::Configuration>(
                        &bytes,
                    ) {
                        Ok(config) => config,
                        Err(_) => {
                            serde_json::from_slice::<mostro_community_api::store::Document>(&bytes)?
                                .config
                                .ok_or("Borrador vacío")?
                        }
                    }
                }
            };
            let origin = args
                .get(2)
                .map(String::as_str)
                .unwrap_or("https://127.0.0.1:10009");
            print!(
                "{}",
                mostro_community_api::config::render_settings(
                    &config,
                    origin,
                    mostro_community_api::daemon::CERT_PATH,
                    mostro_community_api::daemon::MACAROON_PATH,
                )?
            );
            return Ok(());
        }
        if args == ["lnd-tunnel"] {
            return mostro_community_api::tunnel::serve().await;
        }
        if args == ["check-mostro"] {
            let root = PathBuf::from(
                std::env::var("CONFIG_DIR").unwrap_or_else(|_| "./var/config".into()),
            );
            let report =
                mostro_community_api::preflight::report(&root, &Integrations::from_env()).await;
            println!("{}", serde_json::to_string_pretty(&report)?);
            return Ok(());
        }
        if args.len() == 2 && args[0] == "stage-mostro-settings" {
            let root = PathBuf::from(
                std::env::var("CONFIG_DIR").unwrap_or_else(|_| "./var/config".into()),
            );
            let staged = mostro_community_api::staging::stage(&root, &args[1])?;
            println!("{}", serde_json::to_string_pretty(&staged)?);
            return Ok(());
        }
        if args == ["connection-info"] {
            let root = PathBuf::from(
                std::env::var("CONFIG_DIR").unwrap_or_else(|_| "./var/config".into()),
            );
            let store = Store::open(root.clone())?;
            let info = mostro_community_api::connection::get_connection_info(&root, &store);
            println!("{}", serde_json::to_string_pretty(&info)?);
            return Ok(());
        }
        if args == ["check-lnd"] {
            let status = Integrations::from_env().lightning().await;
            println!("{}", serde_json::to_string_pretty(&status)?);
            return if matches!(status["status"].as_str(), Some("online" | "warning")) {
                Ok(())
            } else {
                Err("No se pudo verificar LND".into())
            };
        }
        if args == ["daemon-status"] {
            let root = PathBuf::from(
                std::env::var("CONFIG_DIR").unwrap_or_else(|_| "./var/config".into()),
            );
            let report =
                mostro_community_api::daemon::report(&root, &Integrations::from_env()).await;
            println!("{}", serde_json::to_string_pretty(&report)?);
            return Ok(());
        }
        if !args.is_empty() && (args[0] == "activate-daemon" || args[0] == "start-daemon") {
            let root = PathBuf::from(
                std::env::var("CONFIG_DIR").unwrap_or_else(|_| "./var/config".into()),
            );
            let default_origin = format!(
                "https://{}:{}",
                std::env::var("APP_LIGHTNING_NODE_IP").unwrap_or_else(|_| "10.21.21.9".into()),
                std::env::var("APP_LIGHTNING_NODE_GRPC_PORT").unwrap_or_else(|_| "10009".into())
            );
            let origin = if args.len() >= 2 {
                &args[1]
            } else {
                &default_origin
            };
            let activated = mostro_community_api::daemon::activate(&root, origin)?;
            println!("{}", serde_json::to_string_pretty(&activated)?);
            return Ok(());
        }
        if args == ["deactivate-daemon"] || args == ["stop-daemon"] {
            let root = PathBuf::from(
                std::env::var("CONFIG_DIR").unwrap_or_else(|_| "./var/config".into()),
            );
            mostro_community_api::daemon::deactivate(&root)?;
            println!("Configuración activa de Mostro desactivada.");
            return Ok(());
        }
        if !args.is_empty() && args[0] == "simulate-trade" {
            let root = PathBuf::from(
                std::env::var("CONFIG_DIR").unwrap_or_else(|_| "./var/config".into()),
            );
            let report = mostro_community_api::simulation::run_cli_simulation(&root, &args[1..])
                .map_err(Box::<dyn std::error::Error>::from)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            return Ok(());
        }
        if args == ["derive-public-identity"] {
            let root = PathBuf::from(
                std::env::var("CONFIG_DIR").unwrap_or_else(|_| "./var/config".into()),
            );
            if let Some(npub) = mostro_community_api::identity::inspect(&root)? {
                mostro_community_api::identity::persist_public_key(&root, &npub)
                    .map_err(|e| format!("Error al persistir identidad pública: {e}"))?;
                println!("Identidad pública guardada en mostro.pub: {npub}");
                return Ok(());
            } else {
                return Err("No se encontró identidad privada para derivar la pública".into());
            }
        }
        if args.len() >= 2 && args[0] == "mock-relay" {
            return run_mock_relay(&args[1]).await;
        }
        if !args.is_empty() && args[0] == "auto-backup" {
            let root = PathBuf::from(
                std::env::var("CONFIG_DIR").unwrap_or_else(|_| "./var/config".into()),
            );
            let target_dir = if args.len() >= 2 {
                PathBuf::from(&args[1])
            } else {
                PathBuf::from(
                    std::env::var("BACKUP_OFFSITE_DIR").unwrap_or_else(|_| "/data/backup".into()),
                )
            };
            let passphrase = std::env::var("BACKUP_PASSPHRASE")
                .map_err(|_| "Variable BACKUP_PASSPHRASE no configurada")?;
            let summary = mostro_community_api::backup::run_auto_backup_cycle(
                &root,
                &target_dir,
                7,
                age::secrecy::SecretString::from(passphrase),
            )?;
            println!(
                "{}",
                serde_json::json!({
                    "status": "ok",
                    "path": summary.path.display().to_string(),
                    "revision": summary.revision,
                    "npub": summary.npub
                })
            );
            return Ok(());
        }
        return Err(
            "Uso: mostro-community-api [import-identity|derive-public-identity|export-backup|verify-backup <archivo>|restore-backup <archivo> <directorio-nuevo>|auto-backup [directorio]|stage-mostro-settings <origen-gRPC-LND>|render-settings <community.json|preset:ID> [origen-gRPC-LND]|activate-daemon <origen-gRPC-LND>|deactivate-daemon|daemon-status|simulate-trade [happy-path|dispute-buyer|dispute-seller|cancel] [sats]|mock-relay <bind-addr>|connection-info|check-lnd|check-mostro|lnd-tunnel]".into(),
        );
    }

    let bind = std::env::var("API_BIND").unwrap_or_else(|_| "127.0.0.1:3001".into());
    let root = PathBuf::from(std::env::var("CONFIG_DIR").unwrap_or_else(|_| "./var/config".into()));
    let store = Arc::new(Mutex::new(Store::open(root.clone())?));

    // Configuración del Monitor de Órdenes (Watch Channel)
    let initial_config = store
        .lock()
        .unwrap()
        .document
        .config
        .clone()
        .unwrap_or_else(|| mostro_community_api::config::Configuration {
            community: mostro_community_api::config::Community {
                name: String::new(),
                about: String::new(),
                website: String::new(),
                picture: String::new(),
                contact: String::new(),
                language: "es".into(),
            },
            market: mostro_community_api::config::Market {
                fiat_currencies: vec![],
                min_trade_sats: 1000,
                max_trade_sats: 1000000,
                fee_bps: 0,
                dev_fee_bps: 0,
                max_routing_fee_bps: 0,
            },
            safety: mostro_community_api::config::Safety {
                bond_enabled: false,
                bond_bps: 0,
                base_bond_sats: 0,
                bond_apply_to: mostro_community_api::config::BondApply::Both,
                automatic_timeout_slash: false,
                pow: 0,
                pow_first_contact: 0,
            },
            nostr: mostro_community_api::config::Nostr { relays: vec![] },
            payment_methods: vec![],
        });
    let initial_npub = mostro_community_api::identity::read_public_key(&root)
        .ok()
        .flatten();

    let orders_cache = Arc::new(tokio::sync::RwLock::new(
        mostro_community_api::orders::OrdersCache::new(),
    ));
    let chat_cache = Arc::new(tokio::sync::RwLock::new(
        mostro_community_api::chat::ChatCache::new(),
    ));
    let (monitor_tx, monitor_rx) =
        tokio::sync::watch::channel(mostro_community_api::orders::MonitorCommand {
            config: initial_config,
            npub: initial_npub,
        });

    let notifications = Arc::new(mostro_community_api::notifications::NotificationHub::default());
    let backup_state = Arc::new(tokio::sync::RwLock::new(
        mostro_community_api::backup::AutoBackupState::default(),
    ));
    let rate_limiter = Arc::new(mostro_community_api::RateLimiter::default());

    // A new version may render the saved rules differently (a new mostrod
    // template). Applying it now restarts the daemon once, during the update,
    // instead of on the first unrelated save afterwards.
    match mostro_community_api::daemon::sync_with_recorded_origin(&root) {
        Ok(Some(applied)) if applied.settings_changed => {
            notifications
                .publish(mostro_community_api::notifications::Notification::system_alert(
                    "Configuración del daemon actualizada",
                    "Esta versión genera la configuración de mostrod de otra forma; se aplicó y el daemon se reinició una vez.",
                    "info",
                    None,
                ))
                .await;
        }
        Ok(_) => {}
        Err(reason) => {
            notifications
                .publish(mostro_community_api::notifications::Notification::system_alert(
                    "Activación incompleta",
                    &format!(
                        "Al arrancar no se pudo aplicar la configuración al nodo: {reason}. El daemon sigue con la anterior. Comprueba en la página Nodo Mostro qué revisión está activa y pulsa «Aplicar y reiniciar»."
                    ),
                    "critical",
                    None,
                ))
                .await;
        }
    }

    // Tarea de red en segundo plano (Monitor WS de órdenes con notificaciones)
    tokio::spawn(
        mostro_community_api::orders::monitor_worker_with_notifications(
            orders_cache.clone(),
            monitor_rx,
            mostro_community_api::orders::MonitorTiming::default(),
            Some(notifications.clone()),
        ),
    );

    // Tarea de red en segundo plano (Chat / Mensajería cifrada)
    tokio::spawn(mostro_community_api::chat::chat_worker(
        chat_cache.clone(),
        root.clone(),
        monitor_tx.subscribe(),
        mostro_community_api::orders::MonitorTiming::default(),
    ));

    // Tarea de respaldo automático (Worker periódico)
    let backup_dir = PathBuf::from(
        std::env::var("BACKUP_OFFSITE_DIR").unwrap_or_else(|_| "/data/backup".into()),
    );
    let backup_passphrase = std::env::var("BACKUP_PASSPHRASE")
        .ok()
        .map(age::secrecy::SecretString::from);
    let backup_interval_secs = std::env::var("BACKUP_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(86400); // 24 horas por defecto
    let backup_retention_count = std::env::var("BACKUP_RETENTION_COUNT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(7); // 7 respaldos conservados por defecto

    tokio::spawn(mostro_community_api::backup::auto_backup_worker(
        root.clone(),
        backup_dir,
        std::time::Duration::from_secs(backup_interval_secs),
        backup_retention_count,
        backup_passphrase,
        backup_state.clone(),
        notifications.clone(),
    ));

    // Publicación de la tarjeta de la comunidad en los relays. Con el
    // interruptor apagado, que es como nace, no se conecta a ningún relay.
    let card_publication = mostro_community_api::card_publication::CardPublication::default();
    tokio::spawn(
        mostro_community_api::card_publication::CardPublisher::new(
            store.clone(),
            card_publication.clone(),
            Some(notifications.clone()),
            mostro_community_api::card_publication::PublisherTiming::default(),
        )
        .run(),
    );

    let state = AppState {
        store,
        integrations: Integrations::from_env(),
        orders: orders_cache,
        chat: chat_cache,
        monitor_tx,
        notifications,
        backup_state,
        rate_limiter,
        card_publication,
    };
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    let local_addr = listener.local_addr()?;
    eprintln!("Community API listening on {local_addr}");
    let app = if let Some(directory) = std::env::var_os("STATIC_DIR") {
        use axum::http::{HeaderValue, header};
        use tower_http::{
            services::{ServeDir, ServeFile},
            set_header::SetResponseHeaderLayer,
        };
        let directory = PathBuf::from(directory);
        router(state)
            .fallback_service(ServeDir::new(&directory).not_found_service(ServeFile::new(directory.join("index.html"))))
            .layer(SetResponseHeaderLayer::if_not_present(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
            .layer(SetResponseHeaderLayer::if_not_present(header::CACHE_CONTROL, HeaderValue::from_static("no-store")))
            .layer(SetResponseHeaderLayer::if_not_present(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; base-uri 'none'; frame-ancestors 'self'")))
    } else {
        router(state)
    };
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

async fn run_mock_relay(bind_addr: &str) -> Result<(), Box<dyn std::error::Error>> {
    use futures_util::{SinkExt, StreamExt};
    use nostr::{EventBuilder, Keys, Kind, SecretKey, Tag, Timestamp, ToBech32};
    use tokio_tungstenite::tungstenite::protocol::Message;

    let secret_key = SecretKey::from_slice(&[1u8; 32])?;
    let keys = Keys::new(secret_key);
    let pubkey = keys.public_key();
    let npub = pubkey.to_bech32()?;

    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    let local_addr = listener.local_addr()?;
    println!("MOCK_RELAY_READY npub={npub} url=ws://{local_addr}");

    loop {
        let (stream, _) = listener.accept().await?;
        let keys_clone = keys.clone();
        tokio::spawn(async move {
            if let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await {
                while let Some(Ok(msg)) = ws.next().await {
                    match msg {
                        Message::Text(text) => {
                            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text)
                                && let Some(arr) = val.as_array()
                                && !arr.is_empty()
                                && arr[0] == "REQ"
                                && arr.len() >= 2
                            {
                                let sub_id = arr[1].as_str().unwrap_or("sub");
                                let now = Timestamp::now().as_secs();
                                let tags = vec![
                                    Tag::custom(
                                        nostr::event::tag::TagKind::Custom(
                                            std::borrow::Cow::Borrowed("d"),
                                        ),
                                        vec!["d3b07384-d113-4001-a111-a8e0f1112222".to_string()],
                                    ),
                                    Tag::custom(
                                        nostr::event::tag::TagKind::Custom(
                                            std::borrow::Cow::Borrowed("k"),
                                        ),
                                        vec!["sell".to_string()],
                                    ),
                                    Tag::custom(
                                        nostr::event::tag::TagKind::Custom(
                                            std::borrow::Cow::Borrowed("s"),
                                        ),
                                        vec!["pending".to_string()],
                                    ),
                                    Tag::custom(
                                        nostr::event::tag::TagKind::Custom(
                                            std::borrow::Cow::Borrowed("f"),
                                        ),
                                        vec!["EUR".to_string()],
                                    ),
                                    Tag::custom(
                                        nostr::event::tag::TagKind::Custom(
                                            std::borrow::Cow::Borrowed("amt"),
                                        ),
                                        vec!["250000".to_string()],
                                    ),
                                    Tag::custom(
                                        nostr::event::tag::TagKind::Custom(
                                            std::borrow::Cow::Borrowed("fa"),
                                        ),
                                        vec!["100".to_string()],
                                    ),
                                    Tag::custom(
                                        nostr::event::tag::TagKind::Custom(
                                            std::borrow::Cow::Borrowed("pm"),
                                        ),
                                        vec!["face_to_face".to_string(), "cash".to_string()],
                                    ),
                                    Tag::custom(
                                        nostr::event::tag::TagKind::Custom(
                                            std::borrow::Cow::Borrowed("premium"),
                                        ),
                                        vec!["2".to_string()],
                                    ),
                                    Tag::custom(
                                        nostr::event::tag::TagKind::Custom(
                                            std::borrow::Cow::Borrowed("y"),
                                        ),
                                        vec!["mostro".to_string()],
                                    ),
                                    Tag::custom(
                                        nostr::event::tag::TagKind::Custom(
                                            std::borrow::Cow::Borrowed("z"),
                                        ),
                                        vec!["order".to_string()],
                                    ),
                                    Tag::custom(
                                        nostr::event::tag::TagKind::Custom(
                                            std::borrow::Cow::Borrowed("network"),
                                        ),
                                        vec!["mainnet".to_string()],
                                    ),
                                    Tag::custom(
                                        nostr::event::tag::TagKind::Custom(
                                            std::borrow::Cow::Borrowed("layer"),
                                        ),
                                        vec!["lightning".to_string()],
                                    ),
                                    Tag::custom(
                                        nostr::event::tag::TagKind::Custom(
                                            std::borrow::Cow::Borrowed("expiration"),
                                        ),
                                        vec![(now + 3600).to_string()],
                                    ),
                                    Tag::custom(
                                        nostr::event::tag::TagKind::Custom(
                                            std::borrow::Cow::Borrowed("expires_at"),
                                        ),
                                        vec![(now + 1800).to_string()],
                                    ),
                                ];
                                let event = EventBuilder::new(Kind::from(38383), "")
                                    .tags(tags)
                                    .sign_with_keys(&keys_clone)
                                    .unwrap();
                                let event_msg =
                                    serde_json::json!(["EVENT", sub_id, event]).to_string();
                                let eose_msg = serde_json::json!(["EOSE", sub_id]).to_string();
                                let _ = ws.send(Message::Text(event_msg.into())).await;
                                let _ = ws.send(Message::Text(eose_msg.into())).await;
                            } else if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text)
                                && let Some(arr) = val.as_array()
                                && arr.len() >= 2
                                && arr[0] == "EVENT"
                                && let Some(id) = arr[1].get("id").and_then(|id| id.as_str())
                            {
                                // Takes whatever is published, keeps nothing and
                                // says what it was, for whoever drives the test.
                                println!("MOCK_RELAY_EVENT {}", arr[1]);
                                let ok_msg = serde_json::json!(["OK", id, true, ""]).to_string();
                                let _ = ws.send(Message::Text(ok_msg.into())).await;
                            }
                        }
                        Message::Ping(payload) => {
                            let _ = ws.send(Message::Pong(payload)).await;
                        }
                        Message::Close(_) => break,
                        _ => {}
                    }
                }
            }
        });
    }
}
