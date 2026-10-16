use age::secrecy::SecretString;
use futures_util::{SinkExt, StreamExt};
use mostro_community_api::{
    backup::{apply_retention_policy, export_to_dir, list_backups, verify},
    config::{BondApply, Community, Configuration, Market, Nostr, Safety},
    identity,
    notifications::{Notification, NotificationHub},
    orders::{MonitorCommand, MonitorTiming, OrdersCache, monitor_worker_with_notifications},
    store::Store,
};
use nostr::{EventBuilder, Keys, Kind, SecretKey, Tag, Timestamp, ToBech32};
use std::{fs, os::unix::fs::PermissionsExt, sync::Arc, time::Duration};
use tokio::sync::{RwLock, watch};
use tokio_tungstenite::tungstenite::protocol::Message;

fn dummy_config(relays: Vec<String>) -> Configuration {
    Configuration {
        community: Community {
            name: "Modulo 4 Test Community".into(),
            about: "Testing module 4 notifications and backup".into(),
            website: "https://community.test".into(),
            picture: String::new(),
            contact: "https://contact.test".into(),
            language: "es".into(),
        },
        market: Market {
            fiat_currencies: vec!["EUR".into()],
            min_trade_sats: 1000,
            max_trade_sats: 100000,
            fee_bps: 50,
            dev_fee_bps: 0,
            max_routing_fee_bps: 10,
        },
        safety: Safety {
            bond_enabled: false,
            bond_bps: 0,
            base_bond_sats: 0,
            bond_apply_to: BondApply::Both,
            automatic_timeout_slash: false,
            pow: 0,
            pow_first_contact: 0,
        },
        nostr: Nostr { relays },
        payment_methods: vec![],
    }
}

#[tokio::test]
async fn test_notifications_broadcast_and_sse_stream() {
    let hub = NotificationHub::new(10);
    let mut rx = hub.subscribe();

    hub.publish(Notification::relay_alert(
        "Relay Desconectado",
        "ws://test-relay falló",
        "warning",
        Some(serde_json::json!({"relay": "ws://test-relay"})),
    ))
    .await;

    hub.publish(Notification::dispute_alert(
        "El nodo anuncia la disputa d3b07384-d113-4001-a111-a8e0f1112222, abierta por el comprador",
        Some(serde_json::json!({"amount_sats": 50000})),
    ))
    .await;

    hub.publish(Notification::backup_alert(
        "Backup Completado",
        "Guardado en /data/backup/pre-market.age",
        true,
        Some(serde_json::json!({"revision": 1})),
    ))
    .await;

    // Receive from broadcast receiver
    let n1 = rx.recv().await.unwrap();
    assert_eq!(n1.category, "relay");
    assert_eq!(n1.level, "warning");

    let n2 = rx.recv().await.unwrap();
    assert_eq!(n2.category, "dispute");
    assert_eq!(n2.level, "warning");
    assert_eq!(n2.title, "Disputa abierta");
    assert!(n2.message.contains("d3b07384"));

    let n3 = rx.recv().await.unwrap();
    assert_eq!(n3.category, "backup");
    assert_eq!(n3.level, "info");

    // Verify recent buffer (newest first)
    let recent = hub.get_recent(5).await;
    assert_eq!(recent.len(), 3);
    assert_eq!(recent[0].category, "backup");
    assert_eq!(recent[1].category, "dispute");
    assert_eq!(recent[2].category, "relay");
}

#[tokio::test]
async fn test_auto_backup_cycle_and_retention_policy() {
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let mut store = Store::open(root.path().to_path_buf()).unwrap();
    store
        .save(dummy_config(vec!["wss://relay.damus.io".into()]))
        .unwrap();

    let keys = Keys::new(SecretKey::from_slice(&[42; 32]).unwrap());
    let nsec = keys.secret_key().to_bech32().unwrap();
    let npub = keys.public_key().to_bech32().unwrap();
    identity::import(root.path(), &nsec, &npub).unwrap();

    let offsite = tempfile::tempdir().unwrap();
    let offsite_backup_dir = offsite.path().join("secure_offsite");

    let passphrase = SecretString::from("secure-test-passphrase-module4-123456".to_string());

    // Run 1st backup
    let summary1 = export_to_dir(root.path(), &offsite_backup_dir, passphrase.clone()).unwrap();
    assert_eq!(summary1.npub, npub);
    assert!(summary1.path.exists());

    // Verify encryption and contents
    let verified = verify(&summary1.path, passphrase.clone()).unwrap();
    assert_eq!(verified.npub, npub);
    assert_eq!(verified.revision, 1);

    // Verify directory permissions 0700 and file permissions 0600
    assert_eq!(
        fs::metadata(&offsite_backup_dir)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&summary1.path).unwrap().permissions().mode() & 0o777,
        0o600
    );

    // Simulate multiple historical backups
    let p2 = offsite_backup_dir.join("pre-market-rev1-1000.age");
    let p3 = offsite_backup_dir.join("pre-market-rev1-2000.age");
    let p4 = offsite_backup_dir.join("pre-market-rev1-3000.age");
    fs::copy(&summary1.path, &p2).unwrap();
    fs::copy(&summary1.path, &p3).unwrap();
    fs::copy(&summary1.path, &p4).unwrap();

    let list_before = list_backups(&offsite_backup_dir);
    assert_eq!(list_before.len(), 4);

    // Apply retention policy: retain 2 backups
    let removed = apply_retention_policy(&offsite_backup_dir, 2).unwrap();
    assert_eq!(removed.len(), 2);

    let list_after = list_backups(&offsite_backup_dir);
    assert_eq!(list_after.len(), 2);
    for item in &list_after {
        assert!(item.filename.ends_with(".age"));
        assert!(item.size_bytes > 0);
    }
}

#[tokio::test]
async fn test_dispute_notification_emitted_by_order_monitor() {
    let secret = SecretKey::from_slice(&[11; 32]).unwrap();
    let keys = Keys::new(secret);
    let pubkey = keys.public_key();
    let npub = pubkey.to_bech32().unwrap();

    // Start mock relay
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay_addr = listener.local_addr().unwrap();
    let relay_url = format!("ws://{relay_addr}");

    let keys_clone = keys.clone();
    let relay_handle = tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await
            && let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await
        {
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
                            // What mostrod publishes when a dispute opens: a
                            // kind 38386 event that names the dispute, never
                            // the order. Orders never carry `s=dispute`.
                            let tag = |name: &'static str, values: Vec<String>| {
                                Tag::custom(
                                    nostr::event::tag::TagKind::Custom(std::borrow::Cow::Borrowed(
                                        name,
                                    )),
                                    values,
                                )
                            };
                            let dispute_event = |id: &str, opened_at: u64| {
                                EventBuilder::new(Kind::from(38386), "")
                                    .tags(vec![
                                        tag("d", vec![id.to_string()]),
                                        tag("expiration", vec![(now + 3600).to_string()]),
                                        tag("s", vec!["initiated".into()]),
                                        tag("initiator", vec!["buyer".into()]),
                                        tag("published_at", vec![opened_at.to_string()]),
                                        tag("y", vec!["mostro".into(), "Comunidad".into()]),
                                        tag("z", vec!["dispute".into()]),
                                    ])
                                    .custom_created_at(Timestamp::from(opened_at))
                                    .sign_with_keys(&keys_clone)
                                    .unwrap()
                            };
                            // A dispute opened hours ago, replayed from the
                            // relay's history: it is listed but must not alert.
                            let old =
                                dispute_event("d3b07384-d113-4001-a111-a8e0f1110000", now - 7200);
                            let old_msg = serde_json::json!(["EVENT", sub_id, old]).to_string();
                            let _ = ws.send(Message::Text(old_msg.into())).await;
                            let event = dispute_event("d3b07384-d113-4001-a111-a8e0f1113333", now);
                            let event_msg = serde_json::json!(["EVENT", sub_id, event]).to_string();
                            let eose_msg = serde_json::json!(["EOSE", sub_id]).to_string();
                            let _ = ws.send(Message::Text(event_msg.clone().into())).await;
                            let _ = ws.send(Message::Text(eose_msg.into())).await;
                            // The same event again, as a second relay or a
                            // reconnect would deliver it.
                            let _ = ws.send(Message::Text(event_msg.clone().into())).await;
                            let _ = ws.send(Message::Text(event_msg.into())).await;
                        }
                    }
                    Message::Ping(p) => {
                        let _ = ws.send(Message::Pong(p)).await;
                    }
                    _ => {}
                }
            }
        }
    });

    let hub = Arc::new(NotificationHub::new(10));
    let mut rx = hub.subscribe();

    let cache = Arc::new(RwLock::new(OrdersCache::new()));
    let (_tx, config_rx) = watch::channel(MonitorCommand {
        config: dummy_config(vec![relay_url]),
        npub: Some(npub),
    });

    let timing = MonitorTiming {
        connect_timeout: Duration::from_millis(500),
        reconnect_initial: Duration::from_millis(100),
        reconnect_max: Duration::from_millis(200),
        ping_interval: Duration::from_secs(60),
        ping_timeout: Duration::from_millis(500),
        eose_timeout: Duration::from_millis(500),
        max_future_drift_secs: 60,
    };

    let worker = tokio::spawn(monitor_worker_with_notifications(
        cache.clone(),
        config_rx,
        timing,
        Some(hub.clone()),
    ));

    // Wait for the dispute notification to be received
    let received = tokio::time::timeout(Duration::from_secs(3), rx.recv()).await;
    assert!(received.is_ok(), "Timeout waiting for dispute notification");
    let notif = received.unwrap().unwrap();

    assert_eq!(notif.category, "dispute");
    assert_eq!(notif.level, "warning");
    assert_eq!(notif.title, "Disputa abierta");
    assert!(
        notif
            .message
            .contains("d3b07384-d113-4001-a111-a8e0f1113333, abierta por el comprador")
    );
    assert_eq!(
        notif.details.as_ref().unwrap()["dispute_id"],
        "d3b07384-d113-4001-a111-a8e0f1113333"
    );
    assert_eq!(notif.details.as_ref().unwrap()["initiator"], "buyer");

    // The relay delivered the recent dispute three times and an old one once:
    // exactly one alert, for the recent one, and both disputes are listed.
    let again = tokio::time::timeout(Duration::from_millis(700), rx.recv()).await;
    assert!(
        again.is_err(),
        "a dispute alert was repeated or sent for an old dispute"
    );
    assert_eq!(cache.read().await.to_snapshot().disputes.len(), 2);

    worker.abort();
    relay_handle.abort();
}

#[tokio::test]
async fn test_relay_down_notification_emitted_on_connection_failure() {
    let hub = Arc::new(NotificationHub::new(10));
    let mut rx = hub.subscribe();

    let cache = Arc::new(RwLock::new(OrdersCache::new()));
    // Dead port: guaranteed connection failure
    let dead_relay = "ws://127.0.0.1:1".to_string();

    let secret = SecretKey::from_slice(&[12; 32]).unwrap();
    let npub = Keys::new(secret).public_key().to_bech32().unwrap();

    let (_tx, config_rx) = watch::channel(MonitorCommand {
        config: dummy_config(vec![dead_relay]),
        npub: Some(npub),
    });

    let timing = MonitorTiming {
        connect_timeout: Duration::from_millis(100),
        reconnect_initial: Duration::from_millis(50),
        reconnect_max: Duration::from_millis(100),
        ping_interval: Duration::from_secs(60),
        ping_timeout: Duration::from_millis(500),
        eose_timeout: Duration::from_millis(100),
        max_future_drift_secs: 60,
    };

    let worker = tokio::spawn(monitor_worker_with_notifications(
        cache,
        config_rx,
        timing,
        Some(hub.clone()),
    ));

    // Wait for the relay failure notification
    let received = tokio::time::timeout(Duration::from_secs(3), rx.recv()).await;
    assert!(
        received.is_ok(),
        "Timeout waiting for relay down notification"
    );
    let notif = received.unwrap().unwrap();

    assert_eq!(notif.category, "relay");
    assert!(notif.title.contains("Relay desconectado"));

    worker.abort();
}

#[test]
fn test_rate_limiter_token_replenishment() {
    let limiter = mostro_community_api::RateLimiter::new(10.0, 50.0); // 10 tokens max, 50 tokens/sec
    assert!(limiter.check("client1", 10.0));
    // Now empty
    assert!(!limiter.check("client1", 1.0));

    // Sleep 100ms: 50 * 0.1 = 5 tokens replenished
    std::thread::sleep(Duration::from_millis(100));
    assert!(limiter.check("client1", 4.0));
    // Burning remaining
    assert!(!limiter.check("client1", 5.0));
}
