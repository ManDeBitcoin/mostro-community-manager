use futures_util::{SinkExt, StreamExt};
use mostro_community_api::{
    config::{BondApply, Community, Configuration, Market, Nostr, Safety},
    orders::{
        MonitorCommand, MonitorState, MonitorTiming, OrdersCache, PUBLIC_ORDER_STATUSES,
        SharedOrders, is_closed_status, is_open_dispute_status, is_valid_status, is_valid_uuid,
        parse_and_validate_order_event, parse_dispute_event, parse_node_info_event,
    },
};
use nostr::{
    Event, EventBuilder, Keys, PublicKey, SecretKey, Tag, Timestamp, ToBech32, event::tag::TagKind,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc, watch};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::protocol::Message;

fn valid_config(relays: Vec<String>) -> Configuration {
    Configuration {
        community: Community {
            name: "Test Community".into(),
            about: "Testing orders monitor".into(),
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

async fn spawn_mock_relay() -> (
    String,
    mpsc::Receiver<String>,
    mpsc::Sender<String>,
    JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let url = format!("ws://{}", addr);
    let (req_tx, req_rx) = mpsc::channel::<String>(100);
    let (resp_tx, mut resp_rx) = mpsc::channel::<String>(100);

    let handle = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let req_tx = req_tx.clone();
            if let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await {
                loop {
                    tokio::select! {
                        msg = ws.next() => {
                            match msg {
                                Some(Ok(Message::Text(t))) => {
                                    let _ = req_tx.send(t.to_string()).await;
                                }
                                Some(Ok(Message::Ping(p))) => {
                                    let _ = ws.send(Message::Pong(p)).await;
                                }
                                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                                _ => {}
                            }
                        }
                        out = resp_rx.recv() => {
                            if let Some(text) = out {
                                if ws.send(Message::Text(text.into())).await.is_err() {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                    }
                }
            }
        }
    });

    (url, req_rx, resp_tx, handle)
}

fn build_order_event(
    keys: &Keys,
    uuid: &str,
    kind: &str,
    status: &str,
    created_at_secs: u64,
    expiration_secs: Option<u64>,
    expires_at_secs: Option<u64>,
) -> nostr::Event {
    let mut tags = vec![
        Tag::custom(TagKind::Custom("y".into()), vec!["mostro"]),
        Tag::custom(TagKind::Custom("z".into()), vec!["order"]),
        Tag::identifier(uuid),
        Tag::custom(TagKind::Custom("k".into()), vec![kind]),
        Tag::custom(TagKind::Custom("s".into()), vec![status]),
        Tag::custom(TagKind::Custom("f".into()), vec!["EUR"]),
        Tag::custom(TagKind::Custom("fa".into()), vec!["50"]),
        Tag::custom(TagKind::Custom("amt".into()), vec!["50000"]),
        Tag::custom(TagKind::Custom("pm".into()), vec!["sepa"]),
        Tag::custom(TagKind::Custom("premium".into()), vec!["0"]),
    ];

    if let Some(exp) = expiration_secs {
        tags.push(Tag::custom(
            TagKind::Custom("expiration".into()),
            vec![exp.to_string()],
        ));
    }
    if let Some(eat) = expires_at_secs {
        tags.push(Tag::custom(
            TagKind::Custom("expires_at".into()),
            vec![eat.to_string()],
        ));
    }

    EventBuilder::new(nostr::Kind::Custom(38383), "")
        .tags(tags)
        .custom_created_at(Timestamp::from(created_at_secs))
        .sign_with_keys(keys)
        .unwrap()
}

#[tokio::test]
async fn test_orders_parser_validations() {
    let keys = Keys::new(SecretKey::from_slice(&[1; 32]).unwrap());
    let other_keys = Keys::new(SecretKey::from_slice(&[2; 32]).unwrap());
    let now = Timestamp::now().as_secs();

    // 1. UUID validation helper
    assert!(is_valid_uuid("550e8400-e29b-41d4-a716-446655440000"));
    assert!(!is_valid_uuid("invalid-uuid"));
    assert!(!is_valid_uuid(""));
    assert!(!is_valid_uuid("550e8400-e29b-41d4-a716-44665544000Z"));

    // 2. Valid event parses cleanly
    let uuid = "550e8400-e29b-41d4-a716-446655440000";
    let valid_event = build_order_event(&keys, uuid, "sell", "pending", now, None, None);
    let (summary, is_closed) =
        parse_and_validate_order_event(&valid_event, &keys.public_key(), now, 60).unwrap();
    assert_eq!(summary.id, uuid);
    assert_eq!(summary.kind, "sell");
    assert_eq!(summary.status, "pending");
    assert_eq!(summary.amount_sats, 50000);
    assert_eq!(summary.amount_sats_str, "50000");
    assert!(!is_closed);

    // 3. Invalid signature
    let other_signed = build_order_event(&other_keys, uuid, "sell", "pending", now, None, None);
    let mut tampered = valid_event.clone();
    tampered.sig = other_signed.sig;
    assert!(parse_and_validate_order_event(&tampered, &keys.public_key(), now, 60).is_err());

    // 4. Wrong author
    assert!(
        parse_and_validate_order_event(&valid_event, &other_keys.public_key(), now, 60).is_err()
    );

    // 5. Wrong kind
    let wrong_kind_event = EventBuilder::new(nostr::Kind::Custom(12345), "")
        .tags(vec![
            Tag::custom(TagKind::Custom("y".into()), vec!["mostro"]),
            Tag::custom(TagKind::Custom("z".into()), vec!["order"]),
            Tag::identifier(uuid),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    assert!(
        parse_and_validate_order_event(&wrong_kind_event, &keys.public_key(), now, 60).is_err()
    );

    // 6. Future timestamp beyond drift
    let future_event = build_order_event(&keys, uuid, "sell", "pending", now + 120, None, None);
    assert!(parse_and_validate_order_event(&future_event, &keys.public_key(), now, 60).is_err());

    // 7. Expired by NIP-40
    let expired_event = build_order_event(
        &keys,
        uuid,
        "sell",
        "pending",
        now - 100,
        Some(now - 10),
        None,
    );
    assert!(parse_and_validate_order_event(&expired_event, &keys.public_key(), now, 60).is_err());

    // 8. Expired pending offer by expires_at
    let expired_offer = build_order_event(
        &keys,
        uuid,
        "sell",
        "pending",
        now - 100,
        None,
        Some(now - 10),
    );
    assert!(parse_and_validate_order_event(&expired_offer, &keys.public_key(), now, 60).is_err());

    // 9. Closed status correctly identified
    let canceled_event = build_order_event(&keys, uuid, "sell", "canceled", now, None, None);
    let (_, is_closed_canceled) =
        parse_and_validate_order_event(&canceled_event, &keys.public_key(), now, 60).unwrap();
    assert!(is_closed_canceled);
}

#[tokio::test]
async fn test_monitor_worker_subscribes_and_syncs_with_relay() {
    let keys = Keys::new(SecretKey::from_slice(&[42; 32]).unwrap());
    let npub = keys.public_key().to_bech32().unwrap();
    let (relay_url, mut req_rx, resp_tx, relay_handle) = spawn_mock_relay().await;

    let cache: SharedOrders = Arc::new(RwLock::new(OrdersCache::new()));
    let config = valid_config(vec![relay_url.clone()]);
    let (config_tx, config_rx) = watch::channel(MonitorCommand {
        config,
        npub: Some(npub.clone()),
    });

    let timing = MonitorTiming::test_timing();
    let worker_handle = tokio::spawn(mostro_community_api::orders::monitor_worker(
        cache.clone(),
        config_rx,
        timing,
    ));

    // Server should receive subscription REQ with hex author
    let req_msg = tokio::time::timeout(std::time::Duration::from_millis(500), req_rx.recv())
        .await
        .expect("Timeout esperando REQ del cliente")
        .expect("Canal cerrado");

    assert!(req_msg.contains("\"REQ\""));
    assert!(req_msg.contains("\"mostro_monitor\""));
    assert!(req_msg.contains(&keys.public_key().to_hex()));

    // Verify state before EOSE: is_stale should be true
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    {
        let r = cache.read().await;
        assert_eq!(r.state, MonitorState::Syncing);
        assert!(r.is_stale);
    }

    // Send an order event from mock relay
    let uuid = "550e8400-e29b-41d4-a716-446655440000";
    let now = Timestamp::now().as_secs();
    let order_event = build_order_event(&keys, uuid, "buy", "pending", now, None, None);
    let event_json = serde_json::to_string(&order_event).unwrap();
    let relay_event_msg = format!("[\"EVENT\", \"mostro_monitor\", {}]", event_json);
    resp_tx.send(relay_event_msg).await.unwrap();

    // Send EOSE
    let eose_msg = "[\"EOSE\", \"mostro_monitor\"]".to_string();
    resp_tx.send(eose_msg).await.unwrap();

    // Wait for worker to process EOSE and transition to Live
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    {
        let r = cache.read().await;
        assert_eq!(r.state, MonitorState::Live);
        assert!(!r.is_stale);
        let snapshot = r.to_snapshot();
        assert_eq!(snapshot.orders.len(), 1);
        assert_eq!(snapshot.orders[0].id, uuid);
        assert_eq!(snapshot.orders[0].amount_sats, 50000);
        assert_eq!(snapshot.orders[0].amount_sats_str, "50000");
    }

    // Cleanup
    drop(config_tx);
    worker_handle.abort();
    relay_handle.abort();
}

#[tokio::test]
async fn test_monitor_replacement_tiebreak_and_tombstones() {
    let keys = Keys::new(SecretKey::from_slice(&[7; 32]).unwrap());
    let npub = keys.public_key().to_bech32().unwrap();
    let (relay_url, _req_rx, resp_tx, relay_handle) = spawn_mock_relay().await;

    let cache: SharedOrders = Arc::new(RwLock::new(OrdersCache::new()));
    let config = valid_config(vec![relay_url.clone()]);
    let (config_tx, config_rx) = watch::channel(MonitorCommand {
        config,
        npub: Some(npub),
    });

    let timing = MonitorTiming::test_timing();
    let worker_handle = tokio::spawn(mostro_community_api::orders::monitor_worker(
        cache.clone(),
        config_rx,
        timing,
    ));

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let uuid = "11111111-2222-3333-4444-555555555555";
    let t1 = 1700000000;

    // 1. Initial pending order
    let ev1 = build_order_event(&keys, uuid, "sell", "pending", t1, None, None);
    resp_tx
        .send(format!(
            "[\"EVENT\", \"mostro_monitor\", {}]",
            serde_json::to_string(&ev1).unwrap()
        ))
        .await
        .unwrap();

    // 2. Newer canceled order at t1 + 50
    let ev2 = build_order_event(&keys, uuid, "sell", "canceled", t1 + 50, None, None);
    resp_tx
        .send(format!(
            "[\"EVENT\", \"mostro_monitor\", {}]",
            serde_json::to_string(&ev2).unwrap()
        ))
        .await
        .unwrap();

    // Send EOSE
    resp_tx
        .send("[\"EOSE\", \"mostro_monitor\"]".into())
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    {
        let r = cache.read().await;
        let snap = r.to_snapshot();
        assert_eq!(snap.orders[0].status, "canceled");
    }

    // 3. Replay attack: An older pending event at t1 + 20 arrives after order was canceled at t1 + 50
    let ev_replay = build_order_event(&keys, uuid, "sell", "pending", t1 + 20, None, None);
    resp_tx
        .send(format!(
            "[\"EVENT\", \"mostro_monitor\", {}]",
            serde_json::to_string(&ev_replay).unwrap()
        ))
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    {
        let r = cache.read().await;
        let snap = r.to_snapshot();
        // Tombstone prevented resurrection: still canceled!
        assert_eq!(snap.orders[0].status, "canceled");
    }

    drop(config_tx);
    worker_handle.abort();
    relay_handle.abort();
}

#[tokio::test]
async fn test_monitor_disconnect_preserves_snapshot_with_stale() {
    let keys = Keys::new(SecretKey::from_slice(&[8; 32]).unwrap());
    let npub = keys.public_key().to_bech32().unwrap();
    let (relay_url, _req_rx, resp_tx, relay_handle) = spawn_mock_relay().await;

    let cache: SharedOrders = Arc::new(RwLock::new(OrdersCache::new()));
    let config = valid_config(vec![relay_url.clone()]);
    let (config_tx, config_rx) = watch::channel(MonitorCommand {
        config,
        npub: Some(npub),
    });

    let timing = MonitorTiming::test_timing();
    let worker_handle = tokio::spawn(mostro_community_api::orders::monitor_worker(
        cache.clone(),
        config_rx,
        timing,
    ));

    let uuid = "99999999-8888-7777-6666-555555555555";
    let now = Timestamp::now().as_secs();
    let ev = build_order_event(&keys, uuid, "buy", "pending", now, None, None);
    resp_tx
        .send(format!(
            "[\"EVENT\", \"mostro_monitor\", {}]",
            serde_json::to_string(&ev).unwrap()
        ))
        .await
        .unwrap();
    resp_tx
        .send("[\"EOSE\", \"mostro_monitor\"]".into())
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    {
        let r = cache.read().await;
        assert_eq!(r.state, MonitorState::Live);
        assert!(!r.is_stale);
        assert_eq!(r.to_snapshot().orders.len(), 1);
    }

    // Now kill relay (simulating disconnection)
    relay_handle.abort();
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    {
        let r = cache.read().await;
        // State should be disconnected / reconnecting, but is_stale is true and snapshot is retained!
        assert!(r.is_stale);
        let snap = r.to_snapshot();
        assert_eq!(
            snap.orders.len(),
            1,
            "Snapshot de órdenes debe conservarse tras desconexión"
        );
        assert_eq!(snap.orders[0].id, uuid);
    }

    drop(config_tx);
    worker_handle.abort();
}

#[tokio::test]
async fn test_monitor_multirelay_failover_and_degraded_state() {
    let keys = Keys::new(SecretKey::from_slice(&[9; 32]).unwrap());
    let npub = keys.public_key().to_bech32().unwrap();

    // Relay 1 is completely offline (port closed)
    let offline_url = "ws://127.0.0.1:49999".to_string();
    // Relay 2 is healthy
    let (relay_url_2, _req_rx2, resp_tx2, relay_handle2) = spawn_mock_relay().await;

    let cache: SharedOrders = Arc::new(RwLock::new(OrdersCache::new()));
    let config = valid_config(vec![offline_url, relay_url_2]);
    let (config_tx, config_rx) = watch::channel(MonitorCommand {
        config,
        npub: Some(npub),
    });

    let timing = MonitorTiming::test_timing();
    let worker_handle = tokio::spawn(mostro_community_api::orders::monitor_worker(
        cache.clone(),
        config_rx,
        timing,
    ));

    let uuid = "22222222-3333-4444-5555-666666666666";
    let now = Timestamp::now().as_secs();
    let ev = build_order_event(&keys, uuid, "sell", "pending", now, None, None);
    resp_tx2
        .send(format!(
            "[\"EVENT\", \"mostro_monitor\", {}]",
            serde_json::to_string(&ev).unwrap()
        ))
        .await
        .unwrap();
    resp_tx2
        .send("[\"EOSE\", \"mostro_monitor\"]".into())
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(350)).await;

    {
        let r = cache.read().await;
        // Since relay 1 failed and relay 2 is Live, aggregate state should be Degraded
        assert_eq!(r.state, MonitorState::Degraded);
        assert!(!r.is_stale);
        let snap = r.to_snapshot();
        assert_eq!(snap.orders.len(), 1);
        assert_eq!(snap.orders[0].id, uuid);
        assert_eq!(snap.relays.len(), 2);
    }

    drop(config_tx);
    worker_handle.abort();
    relay_handle2.abort();
}

/// A relay that accepts the connection and then says nothing: the handshake
/// never completes, so every attempt ends in the connect timeout.
async fn spawn_silent_relay() -> (String, JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });
    (format!("ws://{addr}"), handle)
}

#[tokio::test]
async fn a_relay_that_stops_answering_stays_down_between_retries() {
    let keys = Keys::new(SecretKey::from_slice(&[12; 32]).unwrap());
    let npub = keys.public_key().to_bech32().unwrap();
    let (silent_url, silent_handle) = spawn_silent_relay().await;

    let cache: SharedOrders = Arc::new(RwLock::new(OrdersCache::new()));
    let (config_tx, config_rx) = watch::channel(MonitorCommand {
        config: valid_config(vec![silent_url]),
        npub: Some(npub),
    });
    let worker_handle = tokio::spawn(mostro_community_api::orders::monitor_worker(
        cache.clone(),
        config_rx,
        MonitorTiming::test_timing(),
    ));

    // The first attempt is the only one reported as "connecting".
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    while cache.read().await.state != MonitorState::Disconnected {
        assert!(
            tokio::time::Instant::now() < deadline,
            "El primer intento debe acabar en desconectado"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    // Each retry waits at most 100 ms and takes 300 ms to time out, so this
    // window covers about three of them.
    for _ in 0..60 {
        {
            let r = cache.read().await;
            assert_eq!(
                r.state,
                MonitorState::Disconnected,
                "Un reintento no debe ocultar la caída"
            );
            assert!(r.is_stale);
            assert!(r.relay_statuses.values().all(|relay| relay.state
                == MonitorState::Disconnected
                && relay.last_error.is_some()));
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    drop(config_tx);
    worker_handle.abort();
    silent_handle.abort();
}

#[tokio::test]
async fn test_monitor_config_change_clears_cache_and_latest_wins() {
    let keys_a = Keys::new(SecretKey::from_slice(&[10; 32]).unwrap());
    let keys_b = Keys::new(SecretKey::from_slice(&[11; 32]).unwrap());
    let npub_a = keys_a.public_key().to_bech32().unwrap();
    let npub_b = keys_b.public_key().to_bech32().unwrap();

    let (relay_url, _req_rx, resp_tx, relay_handle) = spawn_mock_relay().await;

    let cache: SharedOrders = Arc::new(RwLock::new(OrdersCache::new()));
    let config = valid_config(vec![relay_url.clone()]);
    let (config_tx, config_rx) = watch::channel(MonitorCommand {
        config: config.clone(),
        npub: Some(npub_a),
    });

    let timing = MonitorTiming::test_timing();
    let worker_handle = tokio::spawn(mostro_community_api::orders::monitor_worker(
        cache.clone(),
        config_rx,
        timing,
    ));

    // Send order for author A
    let uuid_a = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
    let now = Timestamp::now().as_secs();
    let ev_a = build_order_event(&keys_a, uuid_a, "buy", "pending", now, None, None);
    resp_tx
        .send(format!(
            "[\"EVENT\", \"mostro_monitor\", {}]",
            serde_json::to_string(&ev_a).unwrap()
        ))
        .await
        .unwrap();
    resp_tx
        .send("[\"EOSE\", \"mostro_monitor\"]".into())
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    {
        let r = cache.read().await;
        assert_eq!(r.to_snapshot().orders.len(), 1);
    }

    // Now update config to author B
    config_tx
        .send(MonitorCommand {
            config: config.clone(),
            npub: Some(npub_b),
        })
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    {
        let r = cache.read().await;
        // Author A orders should be gone from cache
        assert_eq!(r.to_snapshot().orders.len(), 0);
    }

    drop(config_tx);
    worker_handle.abort();
    relay_handle.abort();
}

#[tokio::test]
async fn test_monitor_unconfigured_state_when_missing_identity() {
    let cache: SharedOrders = Arc::new(RwLock::new(OrdersCache::new()));
    let config = valid_config(vec!["ws://127.0.0.1:8080".into()]);
    let (config_tx, config_rx) = watch::channel(MonitorCommand { config, npub: None });

    let timing = MonitorTiming::test_timing();
    let worker_handle = tokio::spawn(mostro_community_api::orders::monitor_worker(
        cache.clone(),
        config_rx,
        timing,
    ));

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    {
        let r = cache.read().await;
        assert_eq!(r.state, MonitorState::Unconfigured);
        assert!(!r.is_stale);
        assert_eq!(r.to_snapshot().orders.len(), 0);
    }

    drop(config_tx);
    worker_handle.abort();
}

// ── Real events published by mostrod v0.19.2 on regtest ───────────────────────

/// Events captured from the official v0.19.2 binary (see the fixture folder).
struct RealEvents {
    node: PublicKey,
    doc: serde_json::Value,
}

impl RealEvents {
    fn load() -> Self {
        let doc: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/mostrod-v0.19.2/public-events.json"))
                .unwrap();
        let node = PublicKey::from_hex(doc["node_pubkey"].as_str().unwrap()).unwrap();
        Self { node, doc }
    }

    fn order(&self, name: &str) -> Vec<Event> {
        serde_json::from_value(self.doc["orders"][name].clone()).unwrap()
    }

    fn list(&self, key: &str) -> Vec<Event> {
        serde_json::from_value(self.doc[key].clone()).unwrap()
    }
}

/// Flows the first capture did not cover (2026-10-04): a take with the invoice
/// attached, a range order, a taken buy order, a taker who walks away, a
/// cooperative cancel, a dispute refunded to the seller, and a node with the
/// anti-abuse bond enabled.
struct MoreRealEvents {
    node: PublicKey,
    bonded_node: PublicKey,
    doc: serde_json::Value,
}

impl MoreRealEvents {
    fn load() -> Self {
        let doc: serde_json::Value = serde_json::from_str(include_str!(
            "fixtures/mostrod-v0.19.2/public-events-2.json"
        ))
        .unwrap();
        let key = |name: &str| PublicKey::from_hex(doc[name].as_str().unwrap()).unwrap();
        Self {
            node: key("node_pubkey"),
            bonded_node: key("bonded_node_pubkey"),
            doc: doc.clone(),
        }
    }

    fn order(&self, name: &str) -> Vec<Event> {
        serde_json::from_value(self.doc["orders"][name].clone()).unwrap()
    }

    fn dispute(&self, name: &str) -> Vec<Event> {
        serde_json::from_value(self.doc["disputes"][name].clone()).unwrap()
    }

    fn bonded_info(&self) -> Event {
        serde_json::from_value(self.doc["bonded_info"].clone()).unwrap()
    }

    /// `status:sats:fiat` of every published revision of an order.
    fn flow(&self, name: &str, node: &PublicKey) -> Vec<String> {
        self.order(name)
            .iter()
            .map(|event| {
                let (summary, _) = parse_and_validate_order_event(event, node, seen_at(event), 60)
                    .expect("a real mostrod event must parse");
                format!(
                    "{}:{}:{}",
                    summary.status,
                    summary.amount_sats,
                    summary.fiat_amount_range.join("-")
                )
            })
            .collect()
    }
}

/// The captured events carry a NIP-40 expiration, so they are parsed as of
/// the moment they were published.
fn seen_at(event: &Event) -> u64 {
    event.created_at.as_secs() + 5
}

#[test]
fn parses_every_order_shape_mostrod_v0_19_2_publishes() {
    let real = RealEvents::load();
    let parse = |event: &Event| {
        parse_and_validate_order_event(event, &real.node, seen_at(event), 60)
            .expect("a real mostrod event must parse")
    };

    // Fixed price: sats set by the maker, premium 0.
    let (fixed, closed) = parse(&real.order("fixed_sats_premium_0")[0]);
    assert_eq!(
        (fixed.kind.as_str(), fixed.status.as_str()),
        ("sell", "pending")
    );
    assert_eq!(fixed.amount_sats, 118_055);
    assert_eq!(fixed.premium, 0);
    assert_eq!(fixed.fiat_code, "USD");
    assert_eq!(fixed.fiat_amount_range, ["100"]);
    assert_eq!(fixed.payment_methods, ["Transferencia bancaria"]);
    assert_eq!(fixed.published_at, Some(1_791_062_849));
    assert_eq!(fixed.expires_at, Some(1_791_149_249));
    assert!(!closed);

    // Market price: amt 0 and the premium the maker asked for.
    let (market, _) = parse(&real.order("market_premium_5")[0]);
    assert_eq!((market.amount_sats, market.premium), (0, 5));
    let (negative, _) = parse(&real.order("market_premium_negative")[0]);
    assert_eq!((negative.amount_sats, negative.premium), (0, -3));
    let (buy, _) = parse(&real.order("buy_market_premium_3")[0]);
    assert_eq!((buy.kind.as_str(), buy.premium), ("buy", 3));

    // Range order: two values in `fa`, amt 0.
    let (range, _) = parse(&real.order("range_premium_2")[0]);
    assert_eq!(range.fiat_amount_range, ["50", "200"]);
    assert_eq!((range.amount_sats, range.premium), (0, 2));
}

#[test]
fn follows_a_real_trade_from_pending_to_success() {
    let real = RealEvents::load();
    let revisions = real.order("full_trade");
    let statuses: Vec<(String, u64, bool)> = revisions
        .iter()
        .map(|event| {
            let (summary, closed) =
                parse_and_validate_order_event(event, &real.node, seen_at(event), 60).unwrap();
            (summary.status, summary.amount_sats, closed)
        })
        .collect();
    // A taken order is published as `in-progress` with the sats the daemon
    // priced at take time; the old whitelist rejected it and froze the order
    // as an open offer.
    assert_eq!(
        statuses,
        [
            ("pending".to_string(), 0, false),
            ("in-progress".to_string(), 56_076, false),
            ("success".to_string(), 56_076, true),
        ]
    );

    let mut cache = OrdersCache::new();
    for event in revisions.iter().take(2) {
        let (summary, closed) =
            parse_and_validate_order_event(event, &real.node, seen_at(event), 60).unwrap();
        assert!(cache.try_insert_event(event.clone(), summary.id, closed, 0));
    }
    let id = "4bba5a60-ed92-4419-b96d-370844ea575e";
    assert_eq!(
        cache.events[id].id, revisions[1].id,
        "in-progress replaced pending"
    );

    let (summary, closed) =
        parse_and_validate_order_event(&revisions[2], &real.node, seen_at(&revisions[2]), 60)
            .unwrap();
    assert!(cache.try_insert_event(revisions[2].clone(), summary.id, closed, 0));
    assert!(cache.closed_orders.contains_key(id));
    // A relay replaying the stale open revisions cannot reopen a closed order.
    assert!(!cache.try_insert_event(revisions[0].clone(), id.into(), false, 0));
    assert!(!cache.try_insert_event(revisions[1].clone(), id.into(), false, 0));
}

#[test]
fn status_rules_match_what_mostrod_publishes() {
    for status in PUBLIC_ORDER_STATUSES {
        assert!(is_valid_status(status), "{status} must be accepted");
    }
    for closed in ["success", "canceled", "completed-by-admin"] {
        assert!(is_closed_status(closed));
    }
    for open in ["pending", "in-progress"] {
        assert!(!is_closed_status(open));
    }
    // A status added by a later daemon is kept, not dropped.
    assert!(is_valid_status("some-future-status"));
    for malformed in ["", "Pending", "in progress", "<script>", &"x".repeat(41)] {
        assert!(
            !is_valid_status(malformed),
            "{malformed:?} must be rejected"
        );
    }

    // An admin-resolved order closes like any other.
    let keys = Keys::new(SecretKey::from_slice(&[1; 32]).unwrap());
    let now = Timestamp::now().as_secs();
    let uuid = "550e8400-e29b-41d4-a716-446655440000";
    let event = build_order_event(&keys, uuid, "sell", "completed-by-admin", now, None, None);
    let (summary, closed) =
        parse_and_validate_order_event(&event, &keys.public_key(), now, 60).unwrap();
    assert_eq!(summary.status, "completed-by-admin");
    assert!(closed);
}

#[test]
fn an_order_without_currency_is_rejected() {
    let keys = Keys::new(SecretKey::from_slice(&[1; 32]).unwrap());
    let now = Timestamp::now().as_secs();
    let event = EventBuilder::new(nostr::Kind::Custom(38383), "")
        .tags(vec![
            Tag::custom(TagKind::Custom("y".into()), vec!["mostro"]),
            Tag::custom(TagKind::Custom("z".into()), vec!["order"]),
            Tag::identifier("550e8400-e29b-41d4-a716-446655440000"),
            Tag::custom(TagKind::Custom("k".into()), vec!["sell"]),
            Tag::custom(TagKind::Custom("s".into()), vec!["pending"]),
            Tag::custom(TagKind::Custom("f".into()), vec!["DOLLARS"]),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    // mostrod always sends a 3-letter `f` tag; inventing a currency would
    // show a false market.
    assert!(parse_and_validate_order_event(&event, &keys.public_key(), now, 60).is_err());
}

#[test]
fn parses_real_dispute_events_and_tracks_their_status() {
    let real = RealEvents::load();
    let events = real.list("disputes");
    let parsed: Vec<_> = events
        .iter()
        .map(|event| parse_dispute_event(event, &real.node, seen_at(event), 60).unwrap())
        .collect();
    let id = "3d62ac23-24b6-4846-8047-4eea1bd22235";
    assert!(parsed.iter().all(|d| d.id == id));
    assert!(
        parsed
            .iter()
            .all(|d| d.initiator.as_deref() == Some("buyer"))
    );
    assert!(parsed.iter().all(|d| d.published_at == Some(1_791_063_450)));
    assert_eq!(
        parsed.iter().map(|d| d.status.as_str()).collect::<Vec<_>>(),
        ["initiated", "in-progress", "settled"]
    );
    assert!(is_open_dispute_status("initiated"));
    assert!(is_open_dispute_status("in-progress"));
    assert!(!is_open_dispute_status("settled"));
    assert!(!is_open_dispute_status("seller-refunded"));

    let mut cache = OrdersCache::new();
    // A worker left over from a previous identity or relay set (another
    // generation) cannot write into the current cache, even news.
    assert_eq!(cache.try_insert_dispute(parsed[0].clone(), 1), None);
    assert!(cache.to_snapshot().disputes.is_empty());
    assert_eq!(cache.try_insert_dispute(parsed[0].clone(), 0), Some(None));
    // The same revision from a second relay is not news.
    assert_eq!(cache.try_insert_dispute(parsed[0].clone(), 0), None);
    assert_eq!(
        cache.try_insert_dispute(parsed[2].clone(), 0),
        Some(Some("initiated".into()))
    );
    // A stale revision never overwrites the resolution.
    assert_eq!(cache.try_insert_dispute(parsed[1].clone(), 0), None);
    let snapshot = cache.to_snapshot();
    assert_eq!(snapshot.disputes.len(), 1);
    assert_eq!(snapshot.disputes[0].status, "settled");

    // An order event is not a dispute, and another author is not the node.
    let order = &real.order("full_trade")[0];
    assert!(parse_dispute_event(order, &real.node, seen_at(order), 60).is_err());
    let stranger = Keys::new(SecretKey::from_slice(&[9; 32]).unwrap()).public_key();
    assert!(parse_dispute_event(&events[0], &stranger, seen_at(&events[0]), 60).is_err());
}

#[test]
fn reads_the_version_the_daemon_announces_about_itself() {
    let real = RealEvents::load();
    let event = &real.list("info")[0];
    let info = parse_node_info_event(event, &real.node, seen_at(event), 60).unwrap();
    assert_eq!(info.mostro_version.as_deref(), Some("0.19.2"));
    assert_eq!(info.protocol_version.as_deref(), Some("2"));
    assert_eq!(info.name.as_deref(), Some("BitMaxis - Regtest"));
    assert_eq!(info.tags["fee"], "0.006");
    assert_eq!(info.tags["min_order_amount"], "1000");
    assert_eq!(info.tags["max_order_amount"], "1000000");
    assert_eq!(info.tags["fiat_currencies_accepted"], "USD");
    assert_eq!(info.tags["bond_enabled"], "false");
    assert_eq!(info.tags["maintenance_mode"], "false");
    assert_eq!(info.tags["lnd_networks"], "regtest");
    // With the bond disabled mostrod sends no bond detail tags.
    assert!(!info.tags.contains_key("bond_amount_pct"));

    let mut cache = OrdersCache::new();
    assert!(
        !cache.try_set_node_info(info.clone(), 7),
        "a worker of another generation cannot set the node info"
    );
    assert!(cache.to_snapshot().node_info.is_none());
    assert!(cache.try_set_node_info(info.clone(), 0));
    assert!(
        !cache.try_set_node_info(info.clone(), 0),
        "same event again"
    );
    let mut newer = info.clone();
    newer.created_at += 300;
    newer.mostro_version = Some("9.9.9".into());
    assert!(
        cache.try_set_node_info(newer, 0),
        "a newer announcement replaces it"
    );
    assert!(
        !cache.try_set_node_info(info.clone(), 0),
        "an older one does not"
    );
    assert!(cache.try_set_node_info(
        {
            let mut latest = info.clone();
            latest.created_at += 600;
            latest
        },
        0
    ));
    assert_eq!(cache.node_info_age_secs(info.created_at + 642), Some(42));
    let snapshot = cache.to_snapshot();
    assert_eq!(
        snapshot.node_info.unwrap().mostro_version.as_deref(),
        Some("0.19.2")
    );

    // Only the node's own info event counts.
    let stranger = Keys::new(SecretKey::from_slice(&[9; 32]).unwrap()).public_key();
    assert!(parse_node_info_event(event, &stranger, seen_at(event), 60).is_err());
    let order = &real.order("full_trade")[0];
    assert!(parse_node_info_event(order, &real.node, seen_at(order), 60).is_err());
}

/// Signs, with a test key and the current time, an event carrying the same
/// tags as a captured one. The captured events have absolute timestamps and a
/// NIP-40 expiration, so the live worker can only be fed fresh copies.
fn resign_now(captured: &Event, keys: &Keys, now: u64) -> Event {
    let tags: Vec<Tag> = captured
        .tags
        .iter()
        .map(|tag| {
            let parts = tag.as_slice();
            let values: Vec<String> = match parts[0].as_str() {
                // The info event is addressed by the node's own pubkey.
                "d" if captured.kind.as_u16() == 38385 => vec![keys.public_key().to_hex()],
                "expiration" => vec![(now + 3600).to_string()],
                "published_at" => vec![now.to_string()],
                _ => parts[1..].to_vec(),
            };
            Tag::custom(TagKind::Custom(parts[0].clone().into()), values)
        })
        .collect();
    EventBuilder::new(captured.kind, "")
        .tags(tags)
        .custom_created_at(Timestamp::from(now))
        .sign_with_keys(keys)
        .unwrap()
}

#[tokio::test]
async fn test_monitor_subscribes_to_orders_info_and_disputes() {
    let real = RealEvents::load();
    let keys = Keys::new(SecretKey::from_slice(&[77; 32]).unwrap());
    let npub = keys.public_key().to_bech32().unwrap();
    let (relay_url, mut req_rx, resp_tx, relay_handle) = spawn_mock_relay().await;

    let cache: SharedOrders = Arc::new(RwLock::new(OrdersCache::new()));
    let (config_tx, config_rx) = watch::channel(MonitorCommand {
        config: valid_config(vec![relay_url]),
        npub: Some(npub),
    });
    let worker = tokio::spawn(mostro_community_api::orders::monitor_worker(
        cache.clone(),
        config_rx,
        MonitorTiming::test_timing(),
    ));

    let req = tokio::time::timeout(std::time::Duration::from_millis(500), req_rx.recv())
        .await
        .expect("REQ")
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&req).unwrap();
    assert_eq!(parsed[2]["kinds"], serde_json::json!([38383, 38385, 38386]));
    assert_eq!(
        parsed[2]["authors"],
        serde_json::json!([keys.public_key().to_hex()])
    );

    // The tag sets are the ones mostrod v0.19.2 really publishes.
    let now = Timestamp::now().as_secs();
    let dispute = resign_now(&real.list("disputes")[2], &keys, now);
    let info = resign_now(&real.list("info")[0], &keys, now);
    let order = resign_now(&real.order("full_trade")[1], &keys, now);
    // Events signed by anyone else are not the node's.
    let stranger = Keys::new(SecretKey::from_slice(&[78; 32]).unwrap());
    let forged_info = resign_now(&real.list("info")[0], &stranger, now + 1);
    for event in [&dispute, &info, &order, &forged_info] {
        resp_tx
            .send(format!(
                r#"["EVENT", "mostro_monitor", {}]"#,
                serde_json::to_string(event).unwrap()
            ))
            .await
            .unwrap();
    }
    resp_tx
        .send(r#"["EOSE", "mostro_monitor"]"#.to_string())
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(120)).await;
    {
        let snapshot = cache.read().await.to_snapshot();
        assert_eq!(snapshot.state, MonitorState::Live);
        let announced = snapshot.node_info.expect("node info from the relay");
        assert_eq!(announced.event_id, info.id.to_hex());
        assert_eq!(announced.mostro_version.as_deref(), Some("0.19.2"));
        assert!(snapshot.node_info_age_secs.is_some_and(|age| age < 30));
        assert_eq!(snapshot.disputes.len(), 1);
        assert_eq!(snapshot.disputes[0].status, "settled");
        assert_eq!(snapshot.disputes[0].event_id, dispute.id.to_hex());
        assert_eq!(snapshot.orders.len(), 1);
        assert_eq!(snapshot.orders[0].status, "in-progress");
        assert_eq!(snapshot.orders[0].amount_sats, 56_076);
    }

    drop(config_tx);
    worker.abort();
    relay_handle.abort();
}

#[test]
fn closed_orders_make_room_for_new_ones() {
    let keys = Keys::new(SecretKey::from_slice(&[1; 32]).unwrap());
    let now = Timestamp::now().as_secs();
    let mut cache = OrdersCache::new();
    let uuid = |i: usize| format!("00000000-0000-0000-0000-{i:012}");

    // 1000 orders: the first 10 closed, the rest open.
    for i in 0..1000 {
        let status = if i < 10 { "success" } else { "pending" };
        let event = build_order_event(
            &keys,
            &uuid(i),
            "sell",
            status,
            now - 5000 + i as u64,
            None,
            None,
        );
        assert!(cache.try_insert_event(event, uuid(i), i < 10, 0));
    }
    assert_eq!(cache.events.len(), 1000);

    // A new order evicts the oldest closed one instead of being dropped.
    let event = build_order_event(&keys, &uuid(5000), "buy", "pending", now, None, None);
    assert!(cache.try_insert_event(event, uuid(5000), false, 0));
    assert_eq!(cache.events.len(), 1000);
    assert!(!cache.events.contains_key(&uuid(0)));
    assert!(cache.events.contains_key(&uuid(1)));
    // Its tombstone stays, so a replayed open revision cannot bring it back.
    assert!(cache.closed_orders.contains_key(&uuid(0)));

    // With only open orders left there is nothing safe to evict.
    for i in 1..10 {
        let event = build_order_event(&keys, &uuid(6000 + i), "buy", "pending", now, None, None);
        assert!(cache.try_insert_event(event, uuid(6000 + i), false, 0));
    }
    let event = build_order_event(&keys, &uuid(7000), "buy", "pending", now, None, None);
    assert!(!cache.try_insert_event(event, uuid(7000), false, 0));
    assert_eq!(cache.events.len(), 1000);
}

/// What mostrod v0.19.2 really publishes for each kind of flow. The public
/// status is coarse: it is what a monitor can know, no more.
#[test]
fn public_status_of_every_real_flow() {
    let real = MoreRealEvents::load();
    let flow = |name: &str| real.flow(name, &real.node);

    // A sell order taken with the buyer's invoice attached skips the wait for
    // the invoice: it is never published as in-progress and stays `pending`
    // on the relays until it closes.
    assert_eq!(
        flow("inline_invoice_take"),
        ["pending:0:30", "success:35184:30"]
    );
    // A range order publishes both bounds; once taken, the chosen amount.
    assert_eq!(
        flow("range_taken"),
        [
            "pending:0:20-60",
            "in-progress:34480:30",
            "success:34480:30"
        ]
    );
    assert_eq!(
        flow("buy_taken"),
        ["pending:0:30", "in-progress:35184:30", "success:35184:30"]
    );
    // A solver's refund and a cooperative cancel look the same in public.
    for canceled in ["admin_canceled", "cooperative_cancel"] {
        assert_eq!(
            flow(canceled),
            ["pending:0:30", "in-progress:35184:30", "canceled:35184:30"]
        );
    }
    // The taker left before sending the invoice: the order is offered again.
    assert_eq!(
        flow("taker_walked_away"),
        [
            "pending:0:30",
            "in-progress:35184:30",
            "pending:0:30",
            "canceled:0:30"
        ]
    );
    // With bonds the published statuses are the same.
    assert_eq!(
        real.flow("bonded_trade", &real.bonded_node),
        ["pending:0:30", "in-progress:35186:30", "success:35186:30"]
    );
    // A maker in reputation mode: the pending revision carries the maker's
    // rating, which does not change how the order is read.
    let bonded = |name: &str| real.flow(name, &real.bonded_node);
    assert_eq!(
        bonded("reputation_trade"),
        ["pending:0:30", "in-progress:35169:30", "success:35169:30"]
    );
    assert_eq!(
        bonded("rated_maker_canceled"),
        ["pending:0:30", "canceled:0:30"]
    );
    let rating_of = |name: &str| -> serde_json::Value {
        let pending = &real.order(name)[0];
        let tag = pending
            .tags
            .iter()
            .find(|tag| tag.kind().to_string() == "rating")
            .expect("a pending order carries the maker rating");
        serde_json::from_str(tag.content().unwrap()).unwrap()
    };
    // Reputation mode adds `since`; a full-privacy maker has no history.
    assert_eq!(rating_of("rated_maker_canceled")[1]["total_reviews"], 1);
    assert!(rating_of("rated_maker_canceled")[1]["since"].is_u64());
    assert!(rating_of("inline_invoice_take")[1].get("since").is_none());
}

#[test]
fn an_order_is_open_again_when_the_taker_walks_away() {
    let real = MoreRealEvents::load();
    let revisions = real.order("taker_walked_away");
    let id = "b231d85e";
    let insert = |cache: &mut OrdersCache, event: &Event| {
        let (summary, closed) =
            parse_and_validate_order_event(event, &real.node, seen_at(event), 60).unwrap();
        assert!(summary.id.starts_with(id));
        (
            cache.try_insert_event(event.clone(), summary.id.clone(), closed, 0),
            summary.id,
        )
    };

    let mut cache = OrdersCache::new();
    let mut order_id = String::new();
    for event in &revisions[..3] {
        let (inserted, full_id) = insert(&mut cache, event);
        assert!(inserted);
        order_id = full_id;
    }
    // pending → in-progress → pending: the newest revision wins and nothing
    // was tombstoned, so the order shows as an open offer again.
    assert_eq!(cache.events[&order_id].id, revisions[2].id);
    assert!(!cache.closed_orders.contains_key(&order_id));
    // A relay replaying the older in-progress revision changes nothing.
    assert!(!insert(&mut cache, &revisions[1]).0);
    assert_eq!(cache.events[&order_id].id, revisions[2].id);

    assert!(insert(&mut cache, &revisions[3]).0);
    assert!(cache.closed_orders.contains_key(&order_id));
    assert!(
        !insert(&mut cache, &revisions[2]).0,
        "a closed order stays closed"
    );
}

#[test]
fn reads_the_bond_policy_a_node_announces() {
    let real = MoreRealEvents::load();
    let event = real.bonded_info();
    let info = parse_node_info_event(&event, &real.bonded_node, seen_at(&event), 60).unwrap();
    assert_eq!(info.mostro_version.as_deref(), Some("0.19.2"));
    // These are the values the Manager rendered into settings.toml for that
    // node: 3 % with a 1000 sats floor for both sides, first-contact PoW 8.
    for (tag, expected) in [
        ("bond_enabled", "true"),
        ("bond_amount_pct", "0.03"),
        ("bond_base_amount_sats", "1000"),
        ("bond_apply_to", "both"),
        ("bond_slash_on_waiting_timeout", "false"),
        ("bond_slash_node_share_pct", "0.5"),
        ("bond_payout_claim_window_days", "15"),
        ("pow", "0"),
        ("pow_first_contact", "8"),
        ("fee", "0.006"),
    ] {
        assert_eq!(
            info.tags.get(tag).map(String::as_str),
            Some(expected),
            "{tag}"
        );
    }
    assert!(parse_node_info_event(&event, &real.node, seen_at(&event), 60).is_err());
}

#[test]
fn a_dispute_refunded_to_the_seller_is_closed() {
    let real = MoreRealEvents::load();
    let parsed: Vec<_> = real
        .dispute("seller_refunded")
        .iter()
        .map(|event| parse_dispute_event(event, &real.node, seen_at(event), 60).unwrap())
        .collect();
    assert_eq!(
        parsed.iter().map(|d| d.status.as_str()).collect::<Vec<_>>(),
        ["initiated", "in-progress", "seller-refunded"]
    );
    assert!(
        parsed
            .iter()
            .all(|d| d.initiator.as_deref() == Some("seller"))
    );
    assert!(is_open_dispute_status(&parsed[1].status));
    assert!(!is_open_dispute_status(&parsed[2].status));

    let bonded: Vec<_> = real
        .dispute("settled_on_bonded_node")
        .iter()
        .map(|event| parse_dispute_event(event, &real.bonded_node, seen_at(event), 60).unwrap())
        .collect();
    assert_eq!(bonded.last().unwrap().status, "settled");
}

/// A pending sell order with the given `fa` and `pm` tags, signed by `keys`.
fn order_event_with(keys: &Keys, uuid: &str, fiat_amounts: &[&str], methods: &[String]) -> Event {
    let custom =
        |name: &'static str, values: Vec<String>| Tag::custom(TagKind::Custom(name.into()), values);
    let tags = vec![
        custom("y", vec!["mostro".into()]),
        custom("z", vec!["order".into()]),
        Tag::identifier(uuid),
        custom("k", vec!["sell".into()]),
        custom("s", vec!["pending".into()]),
        custom("f", vec!["USD".into()]),
        custom("fa", fiat_amounts.iter().map(|v| v.to_string()).collect()),
        custom("amt", vec!["0".into()]),
        custom("pm", methods.to_vec()),
        custom("premium", vec!["0".into()]),
    ];
    EventBuilder::new(nostr::Kind::Custom(38383), "")
        .tags(tags)
        .sign_with_keys(keys)
        .unwrap()
}

/// The `pm` tag is the text the maker typed, copied by the daemon, and `fa`
/// is read from the relays like everything else. Neither reaches the panel
/// longer or stranger than an order needs, from either of the two parsers.
#[test]
fn order_text_from_the_relays_is_cleaned_and_bounded() {
    let keys = Keys::generate();
    let uuid = "d3b07384-d113-4001-a111-a8e0f1112222";
    let now = Timestamp::now().as_secs();
    // What the monitor validates on arrival, and what the snapshot serves.
    let both = |fiat_amounts: &[&str], methods: &[String]| {
        let event = order_event_with(&keys, uuid, fiat_amounts, methods);
        let (validated, _) =
            parse_and_validate_order_event(&event, &keys.public_key(), now + 5, 60).unwrap();
        let mut cache = OrdersCache::new();
        assert!(cache.try_insert_event(event, uuid.to_string(), false, 0));
        let served = cache.to_snapshot().orders.remove(0);
        assert_eq!(validated.payment_methods, served.payment_methods);
        assert_eq!(validated.fiat_amount_range, served.fiat_amount_range);
        served
    };
    let text = |values: &[&str]| -> Vec<String> { values.iter().map(|v| v.to_string()).collect() };

    // What mostrod publishes goes through untouched.
    let order = both(
        &["50", "200"],
        &text(&["Transferencia bancaria", "Pago Móvil"]),
    );
    assert_eq!(order.fiat_amount_range, ["50", "200"]);
    assert_eq!(
        order.payment_methods,
        ["Transferencia bancaria", "Pago Móvil"]
    );

    // A long method is cut, on one line and without invisible marks.
    let long = format!("Banco\nPichincha\u{202e}{}", "x".repeat(300));
    let order = both(
        &["100"],
        &[long, " SEPA ".into(), "  ".into(), "\u{200b}".into()],
    );
    assert_eq!(order.payment_methods.len(), 2, "empty names are dropped");
    let first = &order.payment_methods[0];
    assert!(first.starts_with("Banco Pichinchaxxx"), "{first}");
    assert_eq!(first.chars().count(), 61);
    assert!(first.ends_with('…'));
    assert_eq!(order.payment_methods[1], "SEPA");

    // A list longer than an order needs is cut, and the cut shows.
    let many: Vec<String> = (1..=14).map(|n| format!("Método {n}")).collect();
    let order = both(&["100"], &many);
    assert_eq!(order.payment_methods.len(), 11);
    assert_eq!(order.payment_methods[9], "Método 10");
    assert_eq!(order.payment_methods[10], "…");
    let ten: Vec<String> = many[..10].to_vec();
    assert_eq!(both(&["100"], &ten).payment_methods, ten);

    // An amount is one or two short runs of digits. Anything else is not
    // shown as an amount: the list is empty, which the panel tells apart.
    let methods = text(&["SEPA"]);
    for amounts in [
        vec!["1e9"],
        vec!["-5"],
        vec!["50.00"],
        vec!["1", "2", "3"],
        vec!["1234567890123456"],
        vec!["100", "<b>200</b>"],
        vec![""],
    ] {
        let order = both(&amounts, &methods);
        assert!(order.fiat_amount_range.is_empty(), "{amounts:?}");
    }
    assert_eq!(
        both(&["123456789012345"], &methods).fiat_amount_range,
        ["123456789012345"]
    );
}
