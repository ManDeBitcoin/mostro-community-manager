//! Mediation console: protocol-v2 message decoding against real mostrod v0.19.2 traffic.
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use mostro_community_api::{
    AppState,
    adapters::Integrations,
    chat::{
        self, ChatCache, ChatMessage, build_test_kind14_event, parse_protocol_plaintext,
        summarize_message, summarize_payload,
    },
    config::{BondApply, Community, Configuration, Market, Nostr, Safety},
    identity,
    orders::{DisputeSummary, MonitorCommand, MonitorTiming, OrdersCache},
    router,
    store::Store,
};
use nostr::{EventBuilder, Keys, Kind, SecretKey, Tag, Timestamp, ToBech32, nips::nip04};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::sync::{RwLock, mpsc, watch};
use tokio_tungstenite::tungstenite::protocol::Message;
use tower::ServiceExt;

/// Decrypted protocol messages of one complete trade with a dispute, captured
/// from the official mostrod v0.19.2 binary on regtest (see the fixture folder).
const REAL_MESSAGES: &str = include_str!("fixtures/mostrod-v0.19.2/protocol-messages.jsonl");
/// One complete trade on a node with the anti-abuse bond enabled for both
/// sides and first-contact proof of work, captured the same way.
const BOND_MESSAGES: &str = include_str!("fixtures/mostrod-v0.19.2/protocol-messages-bond.jsonl");
const BOND_ORDER: &str = "6e07c373-b827-465b-8c77-af4b59fe8cfa";
/// One complete trade in reputation mode: every user message carries the
/// inner trade signature and the identity proof. Same bonded node.
const REPUTATION_MESSAGES: &str =
    include_str!("fixtures/mostrod-v0.19.2/protocol-messages-reputation.jsonl");
const REPUTATION_ORDER: &str = "14fd6884-a025-4165-9571-1fa5c773f22c";
/// A disputed trade that a solver cancels while slashing the buyer's bond,
/// and the seller's claim of the counterparty share. Same bonded node.
const SLASH_MESSAGES: &str = include_str!("fixtures/mostrod-v0.19.2/protocol-messages-slash.jsonl");
const SLASH_ORDER: &str = "3d85146b-12d3-461f-a7e5-3e6acacd5634";
const SLASH_DISPUTE: &str = "0c436ff2-ac00-4544-a195-14fb9940ed11";
/// Nine more flows from the same two nodes, in the order the relay received
/// them and with their real timestamps: buy orders, a range order, a take with
/// the invoice attached, cancellations, a take that times out, concurrent
/// takes and a dispute a solver cancels.
const FLOWS: &str = include_str!("fixtures/mostrod-v0.19.2/protocol-flows.json");
const REAL_ORDER: &str = "c359c135-f15b-45c8-8c8d-209f0763bd59";
const REAL_DISPUTE: &str = "3d62ac23-24b6-4846-8047-4eea1bd22235";

fn fixture_keys(byte: u8) -> Keys {
    Keys::new(SecretKey::from_slice(&[byte; 32]).unwrap())
}

fn valid_config(relays: Vec<String>) -> Configuration {
    Configuration {
        community: Community {
            name: "Test Community".into(),
            about: "Testing chat".into(),
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
    tokio::task::JoinHandle<()>,
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
                        msg = futures_util::StreamExt::next(&mut ws) => {
                            match msg {
                                Some(Ok(Message::Text(t))) => {
                                    let _ = req_tx.send(t.to_string()).await;
                                }
                                Some(Ok(Message::Ping(p))) => {
                                    let _ = futures_util::SinkExt::send(&mut ws, Message::Pong(p)).await;
                                }
                                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                                _ => {}
                            }
                        }
                        out = resp_rx.recv() => {
                            if let Some(text) = out {
                                if futures_util::SinkExt::send(&mut ws, Message::Text(text.into())).await.is_err() {
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

/// Re-encrypts the captured plaintexts with test keys and feeds them to the
/// decoder in order. In the capture the admin acted with the node key, so
/// those messages are signed by the node and addressed to the node.
fn replay_real_traffic(node: &Keys, cache: &mut ChatCache) -> (HashMap<String, Keys>, usize) {
    replay_real_traffic_in(node, cache, false, replay_base())
}

/// Timestamp of the first replayed message.
fn replay_base() -> u64 {
    Timestamp::now().as_secs() - 10_000
}

/// `newest_first` delivers the events the way a relay answers a history
/// request: latest event first.
fn replay_real_traffic_in(
    node: &Keys,
    cache: &mut ChatCache,
    newest_first: bool,
    base: u64,
) -> (HashMap<String, Keys>, usize) {
    replay_capture(REAL_MESSAGES, node, cache, newest_first, base)
}

/// Re-encrypts a capture with test keys and feeds it to the decoder.
fn replay_capture(
    capture: &str,
    node: &Keys,
    cache: &mut ChatCache,
    newest_first: bool,
    base: u64,
) -> (HashMap<String, Keys>, usize) {
    let parties: HashMap<String, Keys> = [("seller", 11u8), ("buyer", 12u8)]
        .into_iter()
        .map(|(name, seed)| (name.to_string(), fixture_keys(seed)))
        .collect();
    let mut stored = 0;
    let mut lines: Vec<(usize, &str)> = capture.lines().enumerate().collect();
    if newest_first {
        lines.reverse();
    }
    for (index, line) in lines {
        let row: serde_json::Value = serde_json::from_str(line).unwrap();
        let plaintext = row["plaintext"].as_str().unwrap();
        let party = row["party"].as_str().unwrap();
        let from_daemon = row["dir"] == "daemon->user";
        let created_at = Some(base + index as u64);
        let event = match (party, from_daemon) {
            ("admin", _) => {
                build_test_kind14_event(node, &node.public_key(), plaintext, created_at)
            }
            (_, true) => {
                build_test_kind14_event(node, &parties[party].public_key(), plaintext, created_at)
            }
            (_, false) => {
                build_test_kind14_event(&parties[party], &node.public_key(), plaintext, created_at)
            }
        };
        if chat::process_event(&event, node, cache).unwrap().is_some() {
            stored += 1;
        }
    }
    (parties, stored)
}

#[test]
fn decodes_a_real_v0_19_2_trade_with_dispute() {
    let node = fixture_keys(1);
    let mut cache = ChatCache::new();
    let (parties, stored) = replay_real_traffic(&node, &mut cache);

    // 27 captured messages. Only the maker's new-order request stays out of
    // the thread: it carries no order id yet. The admin's admin-take-dispute
    // names the dispute, which the daemon had already linked to the order.
    assert_eq!(REAL_MESSAGES.lines().count(), 27);
    assert_eq!(stored, 26);
    assert_eq!(cache.order_count(), 1);

    let history = cache.to_history(REAL_ORDER);
    assert_eq!(history.count, 26);
    assert_eq!(history.dispute_id.as_deref(), Some(REAL_DISPUTE));
    assert_eq!(
        cache.order_for_dispute(REAL_DISPUTE).as_deref(),
        Some(REAL_ORDER)
    );

    let actions: Vec<&str> = history
        .messages
        .iter()
        .filter_map(|m| m.action.as_deref())
        .collect();
    for expected in [
        "new-order",
        "take-sell",
        "add-invoice",
        "waiting-buyer-invoice",
        "pay-invoice",
        "waiting-seller-to-pay",
        "buyer-took-order",
        "hold-invoice-payment-accepted",
        "fiat-sent",
        "fiat-sent-ok",
        "dispute",
        "dispute-initiated-by-you",
        "dispute-initiated-by-peer",
        "admin-take-dispute",
        "admin-took-dispute",
        "admin-settle",
        "admin-settled",
        "purchase-completed",
        "rate",
    ] {
        assert!(actions.contains(&expected), "missing action {expected}");
    }
    assert!(history.messages.iter().all(|m| m.kind == 14));
    assert!(
        history
            .messages
            .windows(2)
            .all(|pair| pair[0].created_at <= pair[1].created_at)
    );

    // Roles: the daemon, the two traders and the operator acting as admin.
    let buyer = parties["buyer"].public_key().to_bech32().unwrap();
    let seller = parties["seller"].public_key().to_bech32().unwrap();
    let role_of = |action: &str, sender: &str| {
        history
            .messages
            .iter()
            .find(|m| m.action.as_deref() == Some(action) && m.sender == sender)
            .map(|m| (m.role.clone(), m.is_from_me, m.acknowledged))
    };
    assert_eq!(
        role_of("take-sell", &buyer),
        Some(("user".into(), false, true))
    );
    assert_eq!(
        role_of("fiat-sent", &buyer),
        Some(("user".into(), false, true))
    );
    let node_npub = node.public_key().to_bech32().unwrap();
    assert_eq!(
        role_of("purchase-completed", &node_npub),
        Some(("daemon".into(), true, true))
    );
    // With the node key acting as solver, requests and answers are both
    // node-signed and self-addressed: the action tells them apart.
    for request in ["admin-take-dispute", "admin-settle"] {
        assert_eq!(
            role_of(request, &node_npub),
            Some(("admin".into(), true, true)),
            "{request}"
        );
    }
    for answer in ["admin-took-dispute", "admin-settled", "rate"] {
        assert!(
            history
                .messages
                .iter()
                .filter(|m| m.action.as_deref() == Some(answer))
                .all(|m| m.role == "daemon"),
            "{answer}"
        );
    }
    assert!(
        history
            .messages
            .iter()
            .all(|m| m.sender != seller || m.acknowledged)
    );

    // The summary is rebuilt from known fields: no raw JSON, no full invoice,
    // no signatures.
    let dump = serde_json::to_string(&history).unwrap();
    assert!(!dump.contains("request_id"));
    assert!(!dump.contains("trade_index"));
    assert!(!dump.contains("lnbcrt470890n1p4vzavhpp5tff7"));
    // mostrod hands the solver the hash and the PREIMAGE of the escrow hold
    // invoice in `admin-took-dispute`. With the node key acting as solver the
    // panel decrypts that message; neither value may ever leave the API.
    assert!(REAL_MESSAGES.contains("preimage") && REAL_MESSAGES.contains("d911ec67fb78a65c"));
    for secret in ["d911ec67fb78a65c", "62c41f99509e4b36", "preimage", "hash"] {
        assert!(!dump.contains(secret), "{secret} leaked into the console");
    }
    let hold = history
        .messages
        .iter()
        .find(|m| m.action.as_deref() == Some("pay-invoice") && m.content.contains("factura"))
        .expect("pay-invoice with the hold invoice");
    assert!(hold.content.contains("47373 sats"));
    assert!(hold.content.contains("factura Lightning lnbcrt473730n1…"));
    let opened = history
        .messages
        .iter()
        .find(|m| m.action.as_deref() == Some("dispute-initiated-by-you"))
        .unwrap();
    assert_eq!(opened.content, format!("disputa {REAL_DISPUTE}"));
    assert_eq!(opened.dispute_id.as_deref(), Some(REAL_DISPUTE));
}

#[test]
fn decodes_a_real_trade_with_bonds_for_both_sides() {
    let node = fixture_keys(1);
    let mut cache = ChatCache::new();
    let (parties, stored) = replay_capture(BOND_MESSAGES, &node, &mut cache, false, replay_base());
    // 24 captured messages. The maker asked twice (the first request lacked
    // the first-contact proof of work and the daemon dropped it); neither
    // request names an order yet, so 22 messages form the thread.
    assert_eq!(BOND_MESSAGES.lines().count(), 24);
    assert_eq!(stored, 22);
    let history = cache.to_history(BOND_ORDER);
    assert_eq!(history.count, 22);
    assert_eq!(history.dispute_id, None);

    // With a maker bond the daemon's first answer is the bond request, and
    // the order confirmation only follows once the bond is paid.
    let daemon_actions: Vec<&str> = history
        .messages
        .iter()
        .filter(|m| m.role == "daemon")
        .filter_map(|m| m.action.as_deref())
        .collect();
    assert_eq!(&daemon_actions[..2], ["pay-bond-invoice", "new-order"]);

    // Both parties were asked for a bond. The embedded order of that message
    // carries the bond in `amount`; the console must not present it as the
    // amount of the trade.
    let bonds: Vec<&ChatMessage> = history
        .messages
        .iter()
        .filter(|m| m.action.as_deref() == Some("pay-bond-invoice"))
        .collect();
    assert_eq!(bonds.len(), 2);
    for bond in &bonds {
        assert_eq!(
            bond.content,
            "garantía de 1056 sats · factura Lightning lnbcrt10560n1p…"
        );
    }
    let recipients: Vec<&str> = bonds
        .iter()
        .filter_map(|m| m.recipient.as_deref())
        .collect();
    let seller = parties["seller"].public_key().to_bech32().unwrap();
    let buyer = parties["buyer"].public_key().to_bech32().unwrap();
    assert_eq!(recipients, [seller.as_str(), buyer.as_str()]);

    // The trade itself: escrow of the trade plus the seller's fee, and the
    // buyer's payout minus the buyer's fee.
    let escrow = history
        .messages
        .iter()
        .find(|m| m.action.as_deref() == Some("pay-invoice") && m.content.contains("factura"))
        .unwrap();
    assert!(escrow.content.contains("35292 sats"));
    let payout = history
        .messages
        .iter()
        .find(|m| m.role == "daemon" && m.action.as_deref() == Some("add-invoice"))
        .unwrap();
    assert!(payout.content.contains("35080 sats"));
    let dump = serde_json::to_string(&history).unwrap();
    assert!(
        !dump.contains("lnbcrt10560n1p4vyl"),
        "a full bond invoice leaked"
    );
    assert!(history.messages.iter().all(|m| m.acknowledged));
}

/// The BitMaxis app and every client built on mostro-core send signed tuples:
/// `[message, trade signature, [identity pubkey, identity signature]]`.
#[test]
fn decodes_a_real_trade_in_reputation_mode() {
    let node = fixture_keys(1);
    let mut cache = ChatCache::new();
    let (_, stored) = replay_capture(REPUTATION_MESSAGES, &node, &mut cache, false, replay_base());
    // 27 captured messages; only the maker's first request names no order.
    assert_eq!(REPUTATION_MESSAGES.lines().count(), 27);
    assert_eq!(stored, 26);
    let history = cache.to_history(REPUTATION_ORDER);
    assert_eq!(history.count, 26);

    let tuples: Vec<serde_json::Value> = REPUTATION_MESSAGES
        .lines()
        .map(|line| {
            let row: serde_json::Value = serde_json::from_str(line).unwrap();
            serde_json::from_str(row["plaintext"].as_str().unwrap()).unwrap()
        })
        .collect();
    let signed = tuples
        .iter()
        .filter(|tuple| tuple[1].is_string() && tuple[2].is_array())
        .count();
    // Everything the users sent is signed; nothing the daemon sent is.
    assert_eq!(signed, 8);
    assert_eq!(tuples.iter().filter(|tuple| tuple[1].is_null()).count(), 19);

    let user_actions: Vec<&str> = history
        .messages
        .iter()
        .filter(|m| m.role == "user")
        .filter_map(|m| m.action.as_deref())
        .collect();
    // The seller rated twice: the daemon dropped the first attempt (the trade
    // key was no longer known and the event had no first-contact proof of
    // work). The console shows both requests and a single `rate-received`.
    assert_eq!(
        user_actions,
        [
            "take-sell",
            "add-invoice",
            "fiat-sent",
            "release",
            "rate-user",
            "rate-user",
            "rate-user"
        ]
    );
    let received: Vec<&str> = history
        .messages
        .iter()
        .filter(|m| m.action.as_deref() == Some("rate-received"))
        .map(|m| m.content.as_str())
        .collect();
    assert_eq!(received, ["valoración 5", "valoración 4"]);

    // In reputation mode the maker is told the taker's reputation.
    let peer = history
        .messages
        .iter()
        .find(|m| m.action.as_deref() == Some("pay-invoice") && !m.content.contains("factura"))
        .unwrap();
    assert_eq!(
        peer.content,
        "contraparte · reputación 0.0 en 0 valoraciones"
    );

    // The identity keys behind the trade keys travel inside the encrypted
    // tuple. The console has no use for them and must not show them.
    let dump = serde_json::to_string(&history).unwrap();
    for identity in [
        "d14757da9a3af37eaaed673de4a6ed8314c1f9e12be8605597b30bd93c0d32ac",
        "50b1e698415e5d357613052ac9b19da865e7701d0978744aa5e5a96c21151709",
    ] {
        assert!(REPUTATION_MESSAGES.contains(identity));
        // A shortened form would be a leak too.
        assert!(!dump.contains(&identity[..12]), "an identity key leaked");
    }
}

#[test]
fn decodes_a_real_dispute_with_a_slashed_bond() {
    let node = fixture_keys(1);
    let mut cache = ChatCache::new();
    let (_, stored) = replay_capture(SLASH_MESSAGES, &node, &mut cache, false, replay_base());
    // 32 captured messages; only the maker's first request names no order.
    assert_eq!(SLASH_MESSAGES.lines().count(), 32);
    assert_eq!(stored, 31);
    let history = cache.to_history(SLASH_ORDER);
    assert_eq!(history.count, 31);
    assert_eq!(history.dispute_id.as_deref(), Some(SLASH_DISPUTE));

    let content_of = |role: &str, action: &str| -> Vec<&str> {
        history
            .messages
            .iter()
            .filter(|m| m.role == role && m.action.as_deref() == Some(action))
            .map(|m| m.content.as_str())
            .collect()
    };
    // The solver's decision travels with the cancel.
    assert_eq!(
        content_of("admin", "admin-cancel"),
        ["ejecuta la garantía del comprador"]
    );
    // What follows carries an order whose `amount` is the slashed bond or the
    // counterparty share, never the 35 000 sats of the trade.
    assert_eq!(
        content_of("daemon", "bond-slashed"),
        ["garantía ejecutada: 1054 sats"]
    );
    assert_eq!(
        content_of("daemon", "add-bond-invoice"),
        ["garantía ejecutada: 527 sats a reclamar con una factura"]
    );
    assert_eq!(
        content_of("user", "add-bond-invoice"),
        ["factura Lightning lnbcrt5270n1p4…"]
    );
    assert_eq!(
        content_of("daemon", "bond-invoice-accepted"),
        ["factura de cobro de la garantía aceptada: 527 sats"]
    );
    assert_eq!(
        content_of("daemon", "bond-payout-completed"),
        ["parte de la garantía pagada: 527 sats"]
    );
    let dump = serde_json::to_string(&history).unwrap();
    assert!(!dump.contains("lnbcrt5270n1p4v9z9j"), "an invoice leaked");
}

/// Relays return stored events newest first and the two subscriptions
/// interleave, so the result must not depend on arrival order.
#[test]
fn history_is_the_same_whatever_the_delivery_order() {
    let node = fixture_keys(1);
    // One base for both replays: the comparison below includes timestamps.
    let base = replay_base();
    let mut in_order = ChatCache::new();
    replay_real_traffic_in(&node, &mut in_order, false, base);
    let mut reversed = ChatCache::new();
    let (_, accepted) = replay_real_traffic_in(&node, &mut reversed, true, base);
    // Every message with an order or a dispute is taken in, even those that
    // arrive before the daemon message that gives them a thread.
    assert_eq!(accepted, 26);

    let expected = in_order.to_history(REAL_ORDER);
    let actual = reversed.to_history(REAL_ORDER);
    assert_eq!(actual.count, 26);
    assert_eq!(actual.dispute_id, expected.dispute_id);
    let ids = |history: &chat::ChatHistory| {
        let mut ids: Vec<String> = history.messages.iter().map(|m| m.id.clone()).collect();
        ids.sort();
        ids
    };
    // Event ids differ between the two replays (fresh signatures), so compare
    // what was said, by whom and when.
    let shape = |history: &chat::ChatHistory| {
        let mut rows: Vec<(u64, String, String, bool)> = history
            .messages
            .iter()
            .map(|m| {
                (
                    m.created_at,
                    m.action.clone().unwrap_or_default(),
                    m.role.clone(),
                    m.acknowledged,
                )
            })
            .collect();
        rows.sort();
        rows
    };
    assert_eq!(shape(&actual), shape(&expected));
    assert_eq!(ids(&actual).len(), 26);
    assert_eq!(reversed.orphan_count(), 0);
    assert_eq!(
        reversed.order_for_dispute(REAL_DISPUTE).as_deref(),
        Some(REAL_ORDER)
    );
    // The solver's request names only the dispute; it waited for the link.
    assert!(
        actual
            .messages
            .iter()
            .any(|m| m.action.as_deref() == Some("admin-take-dispute") && m.role == "admin")
    );
}

/// One flow of `protocol-flows.json`, re-encrypted with test keys.
struct Flow {
    order_id: String,
    events: Vec<nostr::Event>,
    /// Messages that name the order or its dispute: what a thread can hold.
    linked: usize,
    /// Every invoice that travelled in the flow.
    invoices: Vec<String>,
}

fn load_flow(name: &str, node: &Keys, base: u64) -> Flow {
    let doc: serde_json::Value = serde_json::from_str(FLOWS).unwrap();
    let flow = &doc["flows"][name];
    assert!(flow.is_object(), "no flow named {name}");
    let party = |name: &str| match name {
        "seller" => fixture_keys(11),
        "buyer" => fixture_keys(12),
        "buyer2" => fixture_keys(13),
        "buyer3" => fixture_keys(14),
        other => panic!("unknown party {other}"),
    };
    let mut events = Vec::new();
    let mut linked = 0;
    let mut invoices = Vec::new();
    for row in flow["messages"].as_array().unwrap() {
        let plaintext = row["plaintext"].as_str().unwrap();
        let name = row["party"].as_str().unwrap();
        let at = Some(base + row["at"].as_u64().unwrap());
        let tuple: serde_json::Value = serde_json::from_str(plaintext).unwrap();
        let inner = tuple[0].as_object().unwrap().values().next().unwrap();
        if inner["id"].is_string() {
            linked += 1;
        }
        if let Some(invoice) = inner["payload"]["payment_request"][1].as_str() {
            invoices.push(invoice.to_string());
        }
        events.push(match (name, row["dir"] == "daemon->user") {
            ("admin", _) => build_test_kind14_event(node, &node.public_key(), plaintext, at),
            (_, true) => build_test_kind14_event(node, &party(name).public_key(), plaintext, at),
            (_, false) => build_test_kind14_event(&party(name), &node.public_key(), plaintext, at),
        });
    }
    Flow {
        order_id: flow["order_id"].as_str().unwrap().to_string(),
        events,
        linked,
        invoices,
    }
}

/// `role:action` of every message of a thread, in timeline order.
fn steps(history: &chat::ChatHistory) -> Vec<String> {
    history
        .messages
        .iter()
        .map(|m| format!("{}:{}", m.role, m.action.as_deref().unwrap_or_default()))
        .collect()
}

fn count_of(steps: &[String], step: &str) -> usize {
    steps.iter().filter(|s| s.as_str() == step).count()
}

/// Every flow decodes whole, in the same order whatever the delivery order,
/// although many of its messages share a second, and no invoice gets through.
#[test]
fn replays_nine_more_real_flows() {
    let node = fixture_keys(1);
    let names = [
        "inline_invoice_take",
        "range_order",
        "buy_order_taken",
        "dispute_admin_cancel",
        "cooperative_cancel",
        "taker_leaves",
        "take_timeout",
        "buy_order_with_invoice",
        "concurrent_takes",
    ];
    let doc: serde_json::Value = serde_json::from_str(FLOWS).unwrap();
    assert_eq!(doc["flows"].as_object().unwrap().len(), names.len());
    for name in names {
        let flow = load_flow(name, &node, replay_base());
        let replay = |newest_first: bool| {
            let mut cache = ChatCache::new();
            let mut stored = 0;
            let mut order: Vec<&nostr::Event> = flow.events.iter().collect();
            if newest_first {
                order.reverse();
            }
            for event in order {
                if chat::process_event(event, &node, &mut cache)
                    .unwrap()
                    .is_some()
                {
                    stored += 1;
                }
            }
            (stored, cache.to_history(&flow.order_id))
        };
        let (stored, history) = replay(false);
        // Only the maker's first request names no order yet.
        assert_eq!(flow.linked, flow.events.len() - 1, "{name}");
        assert_eq!(stored, flow.linked, "{name}");
        assert_eq!(history.count, flow.linked, "{name}");
        assert!(
            history
                .messages
                .windows(2)
                .all(|w| w[0].created_at <= w[1].created_at),
            "{name}: not in time order"
        );
        let ties = history
            .messages
            .windows(2)
            .filter(|w| w[0].created_at == w[1].created_at)
            .count();
        assert!(ties > 0, "{name}: real traffic has same-second messages");

        let (_, reversed) = replay(true);
        let ids = |h: &chat::ChatHistory| -> Vec<String> {
            h.messages.iter().map(|m| m.id.clone()).collect()
        };
        assert_eq!(
            ids(&reversed),
            ids(&history),
            "{name}: order depends on delivery"
        );

        let dump = serde_json::to_string(&history).unwrap();
        assert!(
            !flow.invoices.is_empty() || name == "taker_leaves",
            "{name}"
        );
        for invoice in &flow.invoices {
            assert!(
                !dump.contains(invoice.as_str()),
                "{name}: an invoice leaked"
            );
        }
    }
}

/// What the flows of the integration guide look like in the console.
#[test]
fn real_flows_read_as_the_guide_describes_them() {
    let node = fixture_keys(1);
    let thread = |name: &str| {
        let flow = load_flow(name, &node, replay_base());
        let mut cache = ChatCache::new();
        for event in &flow.events {
            chat::process_event(event, &node, &mut cache).unwrap();
        }
        cache.to_history(&flow.order_id)
    };
    let contents = |history: &chat::ChatHistory, role: &str, action: &str| -> Vec<String> {
        history
            .messages
            .iter()
            .filter(|m| m.role == role && m.action.as_deref() == Some(action))
            .map(|m| m.content.clone())
            .collect()
    };

    // A buy order: the buyer is asked for the invoice only once the seller
    // paid the escrow, and gets a second `add-invoice` that carries nothing
    // but the seller's reputation.
    let buy = thread("buy_order_taken");
    let asked = contents(&buy, "daemon", "add-invoice");
    assert_eq!(asked.len(), 2);
    assert_eq!(
        asked
            .iter()
            .filter(|c| c.starts_with("contraparte"))
            .count(),
        1
    );
    assert_eq!(
        asked
            .iter()
            .filter(|c| c.starts_with("compra · estado waiting-buyer-invoice"))
            .count(),
        1
    );
    let buy_steps = steps(&buy);
    assert_eq!(count_of(&buy_steps, "user:take-buy"), 1);
    assert_eq!(count_of(&buy_steps, "user:add-invoice"), 1);
    assert_eq!(count_of(&buy_steps, "daemon:pay-invoice"), 1);
    // With the invoice already in the order nobody is asked for one.
    let prepaid = steps(&thread("buy_order_with_invoice"));
    assert!(!prepaid.iter().any(|s| s.ends_with(":add-invoice")));
    assert_eq!(count_of(&prepaid, "daemon:pay-bond-invoice"), 2);

    // A take with the invoice attached: one `pay-invoice`, with the invoice.
    let inline = thread("inline_invoice_take");
    let escrow = contents(&inline, "daemon", "pay-invoice");
    assert_eq!(escrow.len(), 1);
    assert!(escrow[0].contains("factura Lightning lnbcrt"));
    assert!(!steps(&inline).iter().any(|s| s.ends_with(":add-invoice")));

    // A range order cannot be taken without an amount.
    let range = thread("range_order");
    assert_eq!(
        contents(&range, "daemon", "cant-do"),
        ["motivo: out_of_range_sats_amount"]
    );
    assert_eq!(count_of(&steps(&range), "user:take-sell"), 2);

    // The taker walks away, or lets the deadline pass: the maker is sent the
    // order again and it is on offer once more.
    let leaves = thread("taker_leaves");
    assert_eq!(count_of(&steps(&leaves), "daemon:new-order"), 2);
    let mut reasons = contents(&leaves, "daemon", "cant-do");
    reasons.sort();
    assert_eq!(
        reasons,
        ["motivo: invalid_order_status", "motivo: invalid_pubkey"]
    );
    let timeout = thread("take_timeout");
    let offers: Vec<u64> = timeout
        .messages
        .iter()
        .filter(|m| m.role == "daemon" && m.action.as_deref() == Some("new-order"))
        .map(|m| m.created_at)
        .collect();
    assert_eq!(offers.len(), 2);
    // The node's deadline for a taken order was 900 s.
    assert!((900..960).contains(&(offers[1] - offers[0])), "{offers:?}");
    assert_eq!(count_of(&steps(&timeout), "daemon:canceled"), 2);
    assert_eq!(count_of(&steps(&timeout), "user:cancel"), 1);

    // Two takers ask for the same bonded order: the one who loses is told.
    let race = steps(&thread("concurrent_takes"));
    assert_eq!(count_of(&race, "daemon:pay-bond-invoice"), 3);
    assert_eq!(count_of(&race, "user:take-sell"), 3);
    assert_eq!(count_of(&race, "daemon:cant-do"), 1);

    // Both parties must ask before an active trade is cancelled.
    let cooperative = steps(&thread("cooperative_cancel"));
    assert_eq!(count_of(&cooperative, "user:cancel"), 2);
    assert_eq!(
        count_of(&cooperative, "daemon:cooperative-cancel-initiated-by-you"),
        1
    );
    assert_eq!(
        count_of(&cooperative, "daemon:cooperative-cancel-initiated-by-peer"),
        1
    );
    assert_eq!(
        count_of(&cooperative, "daemon:cooperative-cancel-accepted"),
        2
    );

    // A solver refunds the seller: the thread carries the dispute and the
    // solver's two requests.
    let refund = thread("dispute_admin_cancel");
    assert!(refund.dispute_id.is_some());
    let refund_steps = steps(&refund);
    assert_eq!(count_of(&refund_steps, "admin:admin-take-dispute"), 1);
    assert_eq!(count_of(&refund_steps, "admin:admin-cancel"), 1);
    assert_eq!(count_of(&refund_steps, "daemon:admin-canceled"), 3);
}

/// The daemon usually answers within the same second as the request, and
/// relays deliver equal timestamps in any order.
#[test]
fn messages_of_the_same_second_have_a_stable_order() {
    let node = fixture_keys(1);
    let seller = fixture_keys(11);
    let buyer = fixture_keys(12);
    let order = "aaaaaaaa-0000-4000-8000-000000000001";
    let base = replay_base();
    let from_daemon = |to: &Keys, action: &str, at: u64| {
        build_test_kind14_event(
            &node,
            &to.public_key(),
            &order_message(action, order),
            Some(at),
        )
    };
    let from_user = |user: &Keys, action: &str, at: u64| {
        build_test_kind14_event(
            user,
            &node.public_key(),
            &order_message(action, order),
            Some(at),
        )
    };
    // Four seconds of a trade, each with several messages:
    //  base     the bond request and the order it guards
    //  base+10  the daemon asks the buyer for an invoice and the app answers
    //           at once; the seller is then asked to pay
    //  base+20  the buyer says the fiat was sent and the daemon tells both
    //  base+30  the seller releases and the daemon sends its closing burst,
    //           which the buyer's app answers with a rating
    let events = [
        from_daemon(&seller, "pay-bond-invoice", base),
        from_daemon(&seller, "new-order", base),
        from_daemon(&buyer, "add-invoice", base + 10),
        from_user(&buyer, "add-invoice", base + 10),
        from_daemon(&seller, "pay-invoice", base + 10),
        from_user(&buyer, "fiat-sent", base + 20),
        from_daemon(&buyer, "fiat-sent-ok", base + 20),
        from_daemon(&seller, "fiat-sent-ok", base + 20),
        from_user(&seller, "release", base + 30),
        from_daemon(&buyer, "released", base + 30),
        from_daemon(&seller, "hold-invoice-payment-settled", base + 30),
        from_daemon(&buyer, "purchase-completed", base + 30),
        from_daemon(&buyer, "rate", base + 30),
        from_user(&buyer, "rate-user", base + 30),
        from_daemon(&buyer, "rate-received", base + 30),
    ];
    let timeline = |arrival: &[usize]| -> Vec<(String, String, String)> {
        let mut cache = ChatCache::new();
        for index in arrival {
            chat::process_event(&events[*index], &node, &mut cache).unwrap();
        }
        cache
            .to_history(order)
            .messages
            .into_iter()
            .map(|m| (m.role, m.action.unwrap_or_default(), m.id))
            .collect()
    };
    let in_order: Vec<usize> = (0..events.len()).collect();
    let expected = timeline(&in_order);
    let steps: Vec<String> = expected
        .iter()
        .map(|(role, action, _)| format!("{role}:{action}"))
        .collect();
    // `released` and `hold-invoice-payment-settled` are one step told to each
    // party, and so are the two `fiat-sent-ok`: the event id orders them.
    let mut closing = steps[9..11].to_vec();
    closing.sort_unstable();
    assert_eq!(
        closing,
        ["daemon:hold-invoice-payment-settled", "daemon:released"]
    );
    assert_eq!(
        [&steps[..9], &steps[11..]].concat(),
        [
            "daemon:pay-bond-invoice",
            "daemon:new-order",
            "daemon:add-invoice",
            "user:add-invoice",
            "daemon:pay-invoice",
            "user:fiat-sent",
            "daemon:fiat-sent-ok",
            "daemon:fiat-sent-ok",
            "user:release",
            "daemon:purchase-completed",
            "daemon:rate",
            "user:rate-user",
            "daemon:rate-received",
        ]
    );
    // Any arrival order gives the same timeline, event by event: newest
    // first, as a relay returns stored events, a user message first, and two
    // scrambles.
    let mut newest_first = in_order.clone();
    newest_first.reverse();
    let arrivals = [
        newest_first,
        vec![5, 0, 1, 2, 3, 4, 6, 7, 8, 9, 10, 11, 12, 13, 14],
        vec![13, 2, 9, 0, 14, 5, 11, 3, 7, 1, 12, 8, 4, 10, 6],
        vec![7, 12, 1, 10, 3, 14, 0, 8, 5, 13, 2, 9, 6, 11, 4],
    ];
    for arrival in &arrivals {
        assert_eq!(timeline(arrival), expected, "arrival {arrival:?}");
    }
}

#[test]
fn a_flood_of_refusals_does_not_push_out_the_real_history() {
    let node = fixture_keys(1);
    let attacker = fixture_keys(3);
    let mut cache = ChatCache::new();
    replay_real_traffic(&node, &mut cache);
    let before: Vec<String> = cache
        .to_history(REAL_ORDER)
        .messages
        .iter()
        .map(|m| m.id.clone())
        .collect();
    assert_eq!(before.len(), 26);

    // Anyone can make the daemon refuse 600 invalid requests about a real
    // order. Each refusal names that order.
    let base = Timestamp::now().as_secs() - 5_000;
    for i in 0..600u64 {
        let event = build_test_kind14_event(
            &node,
            &attacker.public_key(),
            &refusal(REAL_ORDER, "is_not_your_order"),
            Some(base + i),
        );
        chat::process_event(&event, &node, &mut cache).unwrap();
    }
    let history = cache.to_history(REAL_ORDER);
    let refusals = history
        .messages
        .iter()
        .filter(|m| m.action.as_deref() == Some("cant-do"))
        .count();
    assert_eq!(refusals, 20, "only the latest refusals are kept");
    for id in &before {
        assert!(
            history.messages.iter().any(|m| &m.id == id),
            "a real message was evicted"
        );
    }
    assert_eq!(history.count, 46);
    assert_eq!(
        cache.order_for_dispute(REAL_DISPUTE).as_deref(),
        Some(REAL_ORDER)
    );

    // A `cant-do` sent by a user is a user message. Twenty of them, dated
    // after everything else, must not use up the allowance of the daemon's
    // refusals: the next real ones would evict each other.
    let kept: Vec<String> = history
        .messages
        .iter()
        .filter(|m| m.role == "daemon" && m.action.as_deref() == Some("cant-do"))
        .map(|m| m.id.clone())
        .collect();
    assert_eq!(kept.len(), 20);
    for i in 0..20u64 {
        let event = build_test_kind14_event(
            &attacker,
            &node.public_key(),
            &refusal(REAL_ORDER, "invalid_peer"),
            Some(base + 700 + i),
        );
        chat::process_event(&event, &node, &mut cache).unwrap();
    }
    let event = build_test_kind14_event(
        &node,
        &attacker.public_key(),
        &refusal(REAL_ORDER, "not_found"),
        Some(base + 650),
    );
    chat::process_event(&event, &node, &mut cache).unwrap();
    let history = cache.to_history(REAL_ORDER);
    let from_daemon: Vec<&str> = history
        .messages
        .iter()
        .filter(|m| m.role == "daemon" && m.action.as_deref() == Some("cant-do"))
        .map(|m| m.id.as_str())
        .collect();
    // One more real refusal replaced exactly one, the oldest.
    assert_eq!(from_daemon.len(), 20);
    assert_eq!(
        from_daemon
            .iter()
            .filter(|id| kept.iter().any(|k| k == *id))
            .count(),
        19
    );
    assert!(!from_daemon.contains(&kept[0].as_str()));
}

#[test]
fn replaying_the_same_events_does_not_duplicate() {
    let node = fixture_keys(1);
    let buyer = fixture_keys(12);
    let mut cache = ChatCache::new();
    let line = REAL_MESSAGES
        .lines()
        .find(|l| l.contains(r#"\"action\":\"add-invoice\""#) && l.contains("daemon->user"))
        .unwrap();
    let row: serde_json::Value = serde_json::from_str(line).unwrap();
    let event = build_test_kind14_event(
        &node,
        &buyer.public_key(),
        row["plaintext"].as_str().unwrap(),
        None,
    );
    assert!(
        chat::process_event(&event, &node, &mut cache)
            .unwrap()
            .is_some()
    );
    // Same event from a second relay.
    assert!(
        chat::process_event(&event, &node, &mut cache)
            .unwrap()
            .is_none()
    );
    assert_eq!(cache.to_history(REAL_ORDER).count, 1);
}

fn order_message(action: &str, order_id: &str) -> String {
    serde_json::json!([
        {"order": {"version": 2, "request_id": 1, "trade_index": null, "id": order_id, "action": action, "payload": null}},
        null,
        null
    ])
    .to_string()
}

#[test]
fn a_user_cannot_open_a_thread_or_pose_as_a_participant() {
    let node = fixture_keys(1);
    let maker = fixture_keys(2);
    let stranger = fixture_keys(3);
    let mut cache = ChatCache::new();
    let order_id = "11111111-2222-3333-4444-555555555555";

    // Anyone can address a well-formed message to the node about any public
    // order id. Until the daemon answers about that order nothing is shown.
    let early = build_test_kind14_event(
        &stranger,
        &node.public_key(),
        &order_message("fiat-sent", order_id),
        None,
    );
    assert!(
        chat::process_event(&early, &node, &mut cache)
            .unwrap()
            .is_some()
    );
    assert_eq!(cache.order_count(), 0);
    assert_eq!(cache.orphan_count(), 1);
    assert_eq!(cache.to_history(order_id).count, 0);

    // The daemon answers the maker: the thread exists and waiting messages join it.
    let reply = build_test_kind14_event(
        &node,
        &maker.public_key(),
        &order_message("new-order", order_id),
        None,
    );
    assert!(
        chat::process_event(&reply, &node, &mut cache)
            .unwrap()
            .is_some()
    );
    assert_eq!(cache.orphan_count(), 0);
    let history = cache.to_history(order_id);
    assert_eq!(history.count, 2);

    let from_stranger = history.messages.iter().find(|m| !m.is_from_me).unwrap();
    assert!(
        !from_stranger.acknowledged,
        "the daemon never addressed this key"
    );

    // A refusal does not turn its recipient into a participant.
    let refused = build_test_kind14_event(
        &node,
        &stranger.public_key(),
        &refusal(order_id, "is_not_your_order"),
        None,
    );
    assert!(
        chat::process_event(&refused, &node, &mut cache)
            .unwrap()
            .is_some()
    );
    let history = cache.to_history(order_id);
    assert!(
        history
            .messages
            .iter()
            .filter(|m| !m.is_from_me)
            .all(|m| !m.acknowledged)
    );
    let refusal_line = history
        .messages
        .iter()
        .find(|m| m.action.as_deref() == Some("cant-do"))
        .unwrap();
    assert_eq!(refusal_line.content, "motivo: is_not_your_order");
    assert_eq!(refusal_line.variant, "cant-do");

    // The maker, addressed by the daemon, is acknowledged.
    let cancel = build_test_kind14_event(
        &maker,
        &node.public_key(),
        &order_message("cancel", order_id),
        None,
    );
    chat::process_event(&cancel, &node, &mut cache).unwrap();
    let history = cache.to_history(order_id);
    let from_maker = history
        .messages
        .iter()
        .find(|m| m.action.as_deref() == Some("cancel"))
        .unwrap();
    assert!(from_maker.acknowledged);
}

fn refusal(order_id: &str, reason: &str) -> String {
    serde_json::json!([
        {"cant-do": {"version": 2, "request_id": 1, "trade_index": null, "id": order_id, "action": "cant-do", "payload": {"cant_do": reason}}},
        null,
        null
    ])
    .to_string()
}

#[test]
fn a_refusal_cannot_open_a_thread() {
    let node = fixture_keys(1);
    let attacker = fixture_keys(3);
    let mut cache = ChatCache::new();
    // Anyone can make the daemon answer `cant-do` about an order id of their
    // choosing: the refusal echoes the id of the request. If refusals opened
    // threads, a thousand of them would push every real order out of the
    // console.
    for i in 0..5u64 {
        let invented = format!("12121212-0000-0000-0000-{i:012}");
        let event = build_test_kind14_event(
            &node,
            &attacker.public_key(),
            &refusal(&invented, "not_found"),
            None,
        );
        assert_eq!(chat::process_event(&event, &node, &mut cache), Ok(None));
        assert_eq!(cache.to_history(&invented).count, 0);
    }
    assert_eq!(cache.order_count(), 0);
    assert_eq!(cache.orphan_count(), 0);
}

#[test]
fn threads_with_a_dispute_are_the_last_to_be_evicted() {
    let node = fixture_keys(1);
    let maker = fixture_keys(2);
    let mut cache = ChatCache::new();
    let base = Timestamp::now().as_secs() - 100_000;
    let disputed = "abababab-0000-0000-0000-000000000001";
    let dispute_id = "cdcdcdcd-0000-0000-0000-000000000001";

    // The oldest thread of all is the one with an open dispute.
    let opened = serde_json::json!([
        {"order": {"version": 2, "request_id": 1, "trade_index": null, "id": disputed, "action": "dispute-initiated-by-you", "payload": {"dispute": [dispute_id, null]}}},
        null,
        null
    ])
    .to_string();
    let event = build_test_kind14_event(&node, &maker.public_key(), &opened, Some(base));
    chat::process_event(&event, &node, &mut cache).unwrap();

    for i in 0..1005u64 {
        let other = format!("bbbbbbbb-0000-0000-0000-{i:012}");
        let event = build_test_kind14_event(
            &node,
            &maker.public_key(),
            &order_message("new-order", &other),
            Some(base + 10 + i),
        );
        chat::process_event(&event, &node, &mut cache).unwrap();
    }
    assert_eq!(cache.order_count(), 1000);
    assert_eq!(cache.to_history(disputed).count, 1);
    assert_eq!(
        cache.order_for_dispute(dispute_id).as_deref(),
        Some(disputed)
    );
    // The stalest threads without a dispute left instead.
    assert_eq!(
        cache
            .to_history("bbbbbbbb-0000-0000-0000-000000000000")
            .count,
        0
    );
    assert_eq!(
        cache
            .to_history("bbbbbbbb-0000-0000-0000-000000001004")
            .count,
        1
    );
}

#[test]
fn backdated_user_messages_are_not_part_of_the_history() {
    let node = fixture_keys(1);
    let maker = fixture_keys(2);
    let stranger = fixture_keys(3);
    let mut cache = ChatCache::new();
    let order_id = "56565656-0000-0000-0000-000000000001";
    let opened_at = Timestamp::now().as_secs() - 5_000;

    let opened = build_test_kind14_event(
        &node,
        &maker.public_key(),
        &order_message("new-order", order_id),
        Some(opened_at),
    );
    chat::process_event(&opened, &node, &mut cache).unwrap();

    // The daemon only acts on requests a few seconds old, so nothing a user
    // sent can predate the order. A message dated an hour before it is an
    // attempt to plant history, whoever signs it.
    for (keys, when) in [(&stranger, opened_at - 3_600), (&maker, opened_at - 3_600)] {
        let planted = build_test_kind14_event(
            keys,
            &node.public_key(),
            &order_message("fiat-sent", order_id),
            Some(when),
        );
        chat::process_event(&planted, &node, &mut cache).unwrap();
    }
    assert_eq!(cache.to_history(order_id).count, 1);

    // A message sent after the order exists is shown.
    let genuine = build_test_kind14_event(
        &maker,
        &node.public_key(),
        &order_message("cancel", order_id),
        Some(opened_at + 30),
    );
    chat::process_event(&genuine, &node, &mut cache).unwrap();
    let history = cache.to_history(order_id);
    assert_eq!(history.count, 2);
    assert_eq!(history.messages[1].action.as_deref(), Some("cancel"));

    // The same events delivered again by another relay change nothing, even
    // after any amount of unrelated traffic.
    for event in [&opened, &genuine] {
        assert_eq!(chat::process_event(event, &node, &mut cache), Ok(None));
    }
    assert_eq!(cache.to_history(order_id).count, 2);
}

#[test]
fn ignores_everything_that_is_not_a_protocol_v2_message() {
    let node = fixture_keys(1);
    let trader = fixture_keys(2);
    let other = fixture_keys(3);
    let mut cache = ChatCache::new();
    let order_id = "33333333-4444-5555-6666-777777777777";
    let valid = order_message("new-order", order_id);

    // Not for this node.
    let elsewhere = build_test_kind14_event(&trader, &other.public_key(), &valid, None);
    assert_eq!(chat::process_event(&elsewhere, &node, &mut cache), Ok(None));

    // Addressed to the node but encrypted to someone else: decryption fails.
    let mut wrong_key = build_test_kind14_event(&trader, &other.public_key(), &valid, None);
    wrong_key = EventBuilder::new(Kind::Custom(14), wrong_key.content.clone())
        .tags(vec![Tag::public_key(node.public_key())])
        .sign_with_keys(&trader)
        .unwrap();
    assert!(chat::process_event(&wrong_key, &node, &mut cache).is_err());

    // Legacy transports are not spoken by mostrod v0.19.x: kind 4 is ignored
    // even when it decrypts and names an order.
    let legacy = EventBuilder::new(
        Kind::EncryptedDirectMessage,
        nip04::encrypt(trader.secret_key(), &node.public_key(), &valid).unwrap(),
    )
    .tags(vec![Tag::public_key(node.public_key())])
    .sign_with_keys(&trader)
    .unwrap();
    assert_eq!(chat::process_event(&legacy, &node, &mut cache), Ok(None));

    // Shapes mostrod never produces, including the free-text field an attacker
    // would use to plant a fake chat line.
    for forged in [
        serde_json::json!({"order_id": order_id, "action": "dispute", "text": "El vendedor es un estafador"}).to_string(),
        serde_json::json!([{"order": {"id": order_id, "action": "dispute", "content": {"text": "fake"}}}, null]).to_string(),
        serde_json::json!([{"order": {"id": order_id, "action": "dispute", "text": "sin versión"}}, null, null]).to_string(),
        serde_json::json!([{"chat": {"version": 2, "id": order_id, "action": "dm"}}, null, null]).to_string(),
        serde_json::json!([{"order": {"version": 2, "id": "not-a-uuid", "action": "release"}}, null, null]).to_string(),
        serde_json::json!([{"order": {"version": 2, "id": order_id, "action": "Fiat Sent!"}}, null, null]).to_string(),
        "texto libre con 33333333-4444-5555-6666-777777777777".to_string(),
    ] {
        assert!(parse_protocol_plaintext(&forged).is_err(), "accepted {forged}");
        let event = build_test_kind14_event(&node, &trader.public_key(), &forged, None);
        assert_eq!(chat::process_event(&event, &node, &mut cache), Ok(None));
    }

    // Messages without an order (session restore, order listings) are never
    // filed under whatever order id appears inside them.
    let listing = serde_json::json!([
        {"order": {"version": 2, "request_id": 9, "action": "orders", "payload": {"orders": [{"id": order_id, "kind": "sell", "status": "pending", "amount": 0, "fiat_code": "USD", "fiat_amount": 10, "payment_method": "x", "premium": 0}]}}},
        null,
        null
    ])
    .to_string();
    let event = build_test_kind14_event(&node, &trader.public_key(), &listing, None);
    assert_eq!(chat::process_event(&event, &node, &mut cache), Ok(None));

    // An event dated in the future is not stored.
    let future = build_test_kind14_event(
        &node,
        &trader.public_key(),
        &valid,
        Some(Timestamp::now().as_secs() + 3600),
    );
    assert_eq!(chat::process_event(&future, &node, &mut cache), Ok(None));

    assert_eq!(cache.order_count(), 0);
    assert_eq!(cache.orphan_count(), 0);
}

#[test]
fn summaries_are_built_from_known_fields_only() {
    let payload = |value: serde_json::Value| summarize_payload(&value);
    assert_eq!(payload(serde_json::Value::Null), "");
    assert_eq!(
        payload(
            serde_json::json!({"order": {"kind": "sell", "status": "pending", "amount": 0, "fiat_code": "USD", "fiat_amount": 100, "payment_method": "Transferencia bancaria", "premium": 5}})
        ),
        "venta · estado pending · 100 USD · a precio de mercado · prima +5 % · Transferencia bancaria"
    );
    assert_eq!(
        payload(
            serde_json::json!({"order": {"kind": "buy", "status": "pending", "amount": 0, "fiat_code": "USD", "min_amount": 50, "max_amount": 200, "fiat_amount": 0, "payment_method": "Zelle", "premium": -3}})
        ),
        "compra · estado pending · 50–200 USD · a precio de mercado · prima -3 % · Zelle"
    );
    assert_eq!(
        payload(
            serde_json::json!({"payment_request": [null, "lnbc1pvjluezsp5zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zygs", 1500]})
        ),
        "factura Lightning lnbc1pvjluezsp… · 1500 sats"
    );
    assert_eq!(
        payload(serde_json::json!({"cant_do": "invalid_parameters"})),
        "motivo: invalid_parameters"
    );
    assert_eq!(
        payload(serde_json::json!({"rating_user": 5})),
        "valoración 5"
    );
    assert_eq!(
        payload(
            serde_json::json!({"peer": {"pubkey": "72ac6257298054706845515ce7f87ce3ab9f8f6e7b0a2247465435b00a896e08", "reputation": {"rating": 4.5, "reviews": 12, "operating_days": 30}}})
        ),
        "contraparte 72ac62572980… · reputación 4.5 en 12 valoraciones"
    );
    // Bond payloads.
    assert_eq!(
        payload(
            serde_json::json!({"bond_resolution": {"slash_seller": false, "slash_buyer": true}})
        ),
        "ejecuta la garantía del comprador"
    );
    assert_eq!(
        payload(
            serde_json::json!({"bond_resolution": {"slash_seller": false, "slash_buyer": false}})
        ),
        "sin ejecutar garantías"
    );
    assert_eq!(
        payload(
            serde_json::json!({"bond_payout_request": {"order": {"id": "x", "amount": 528}, "slashed_at": 1791000000}})
        ),
        "garantía ejecutada: 528 sats a reclamar con una factura"
    );
    assert_eq!(
        payload(
            serde_json::json!({"payment_failed": {"payment_attempts": 3, "payment_retries_interval": 60}})
        ),
        "pago al comprador fallido: 3 intentos, uno cada 60 s"
    );
    // The same payload means different things under different actions.
    let bond_request = serde_json::json!({"payment_request": [
        {"kind": "sell", "status": "pending", "amount": 1056, "fiat_code": "USD", "fiat_amount": 30, "payment_method": "Transferencia bancaria", "premium": 0},
        "lnbcrt10560n1p4vylxyzxyzxyzxyzxyzxyzxyzxyz",
        null
    ]});
    assert_eq!(
        summarize_message("pay-bond-invoice", &bond_request, true),
        "garantía de 1056 sats · factura Lightning lnbcrt10560n1p…"
    );
    assert_eq!(
        summarize_message("pay-invoice", &bond_request, true),
        "venta · estado pending · 30 USD · 1056 sats · Transferencia bancaria · factura Lightning lnbcrt10560n1p…"
    );
    // Only the daemon speaks with the daemon's words: the same action sent by
    // a user gets the plain summary of its payload.
    let payout = serde_json::json!({"order": {"kind": "sell", "amount": 527, "fiat_code": "USD", "fiat_amount": 30}});
    assert_eq!(
        summarize_message("bond-payout-completed", &payout, true),
        "parte de la garantía pagada: 527 sats"
    );
    assert_eq!(
        summarize_message("bond-payout-completed", &payout, false),
        "venta · 30 USD · 527 sats"
    );
    assert!(!summarize_message("pay-bond-invoice", &bond_request, false).contains("garantía"));
    // Text a sender controls is shown on one line, without characters that
    // would change how the timeline is laid out.
    let crafted = payload(serde_json::json!({"order": {
        "kind": "sell", "amount": 0, "fiat_code": "USD", "fiat_amount": 30,
        "payment_method": "Banco\n\u{202E}odneuqes\u{200B} real\ty\r\nmás"
    }}));
    assert_eq!(
        crafted,
        "venta · 30 USD · a precio de mercado · Banco odneuqes real y  más"
    );
    assert!(!crafted.chars().any(|c| c.is_control()));
    // An invoice is ASCII letters and digits; nothing else gets through.
    assert_eq!(
        payload(
            serde_json::json!({"payment_request": [null, "lnbc1\n<b>\u{202E}xy z0123456789abcdef", null]})
        ),
        "factura Lightning lnbc1bxyz01234…"
    );
    // Unknown payloads show their name, never their content.
    assert_eq!(
        payload(serde_json::json!({"restore_data": {"orders": [], "secret": "x"}})),
        "datos: restore_data"
    );
    // A direct-message text is the only free text shown, and it is capped.
    let long = "a".repeat(2000);
    let shown = payload(serde_json::json!({"text_message": long}));
    assert_eq!(shown.chars().count(), 501);
}

#[test]
fn caches_are_bounded() {
    let node = fixture_keys(1);
    let maker = fixture_keys(2);
    let spammer = fixture_keys(3);
    let mut cache = ChatCache::new();
    let order_id = "44444444-5555-6666-7777-888888888888";
    let base = Timestamp::now().as_secs() - 100_000;

    let opened = build_test_kind14_event(
        &node,
        &maker.public_key(),
        &order_message("new-order", order_id),
        Some(base),
    );
    chat::process_event(&opened, &node, &mut cache).unwrap();
    let genuine = build_test_kind14_event(
        &maker,
        &node.public_key(),
        &order_message("cancel", order_id),
        Some(base + 1),
    );
    chat::process_event(&genuine, &node, &mut cache).unwrap();

    // A flood from a key the daemon never answered cannot push out the
    // participant's message nor grow without limit.
    for i in 0..400u64 {
        let junk = build_test_kind14_event(
            &spammer,
            &node.public_key(),
            &order_message("fiat-sent", order_id),
            Some(base + 2 + i),
        );
        chat::process_event(&junk, &node, &mut cache).unwrap();
    }
    let history = cache.to_history(order_id);
    let user_messages: Vec<&ChatMessage> =
        history.messages.iter().filter(|m| !m.is_from_me).collect();
    assert_eq!(user_messages.len(), 300);
    assert!(
        user_messages
            .iter()
            .any(|m| m.action.as_deref() == Some("cancel") && m.acknowledged)
    );

    // Orphans (messages about orders the daemon never mentioned) are capped
    // per sender, so one key cannot flush everyone else's pending requests...
    for i in 0..120u64 {
        let unknown = format!("99999999-0000-0000-0000-{i:012}");
        let junk = build_test_kind14_event(
            &spammer,
            &node.public_key(),
            &order_message("release", &unknown),
            Some(base + 1000 + i),
        );
        chat::process_event(&junk, &node, &mut cache).unwrap();
    }
    assert_eq!(cache.orphan_count(), 50);
    assert_eq!(cache.order_count(), 1);
    // ...and in total, whatever the number of keys.
    for i in 0..2100u64 {
        let message = ChatMessage {
            id: format!("{i:064x}"),
            order_id: format!("88888888-0000-0000-0000-{i:012}"),
            sender: format!("npub-{i}"),
            recipient: None,
            created_at: base + 2000 + i,
            kind: 14,
            action: Some("release".into()),
            content: String::new(),
            is_from_me: false,
            role: "user".into(),
            variant: "order".into(),
            dispute_id: None,
            acknowledged: false,
        };
        assert!(cache.insert_user_message(message, format!("{i:064x}")));
    }
    assert_eq!(cache.orphan_count(), 2000);
    assert_eq!(cache.order_count(), 1);

    // Threads are capped at 1000 orders: the stalest one leaves first.
    for i in 0..1000u64 {
        let other = format!("aaaaaaaa-0000-0000-0000-{i:012}");
        let event = build_test_kind14_event(
            &node,
            &maker.public_key(),
            &order_message("new-order", &other),
            Some(base + 5000 + i),
        );
        chat::process_event(&event, &node, &mut cache).unwrap();
    }
    assert_eq!(cache.order_count(), 1000);
    assert_eq!(cache.to_history(order_id).count, 0, "oldest thread evicted");
}

fn test_app(root: &std::path::Path, orders: OrdersCache, chat_cache: ChatCache) -> axum::Router {
    let initial_config: Configuration =
        serde_json::from_str(include_str!("fixtures/community.json")).unwrap();
    let (tx, _rx) = watch::channel(MonitorCommand {
        config: initial_config,
        npub: None,
    });
    router(AppState::new(
        Arc::new(Mutex::new(Store::open(root.into()).unwrap())),
        Integrations::default(),
        Arc::new(RwLock::new(orders)),
        Arc::new(RwLock::new(chat_cache)),
        tx,
    ))
}

async fn get_json(app: &axum::Router, uri: &str, protected: bool) -> (StatusCode, Vec<u8>) {
    let mut request = Request::builder().method("GET").uri(uri);
    if protected {
        request = request.header("x-requested-with", "mostro-community");
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, body.to_vec())
}

#[tokio::test]
async fn test_http_chat_endpoint_contract() {
    let dir = tempfile::tempdir().unwrap();
    let node = fixture_keys(1);
    identity::import(
        dir.path(),
        &node.secret_key().to_bech32().unwrap(),
        &node.public_key().to_bech32().unwrap(),
    )
    .unwrap();
    let mut cache = ChatCache::new();
    replay_real_traffic(&node, &mut cache);
    let app = test_app(dir.path(), OrdersCache::new(), cache);

    // 1. Missing protection header -> 403 Forbidden
    let (status, _) = get_json(&app, &format!("/api/chat/{REAL_ORDER}"), false).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // 2. Invalid UUID format -> 400 Bad Request
    let (status, _) = get_json(&app, "/api/chat/not-a-valid-uuid", true).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 3. Valid UUID with no messages -> 200 OK with empty count
    let other_uuid = "00000000-0000-0000-0000-000000000000";
    let (status, body) = get_json(&app, &format!("/api/chat/{other_uuid}"), true).await;
    assert_eq!(status, StatusCode::OK);
    let history: chat::ChatHistory = serde_json::from_slice(&body).unwrap();
    assert_eq!(history.order_id, other_uuid);
    assert_eq!(history.count, 0);
    assert!(history.messages.is_empty());
    assert_eq!(history.dispute_id, None);

    // 4. Valid UUID with messages -> 200 OK with history
    let (status, body) = get_json(&app, &format!("/api/chat/{REAL_ORDER}"), true).await;
    assert_eq!(status, StatusCode::OK);
    let history: chat::ChatHistory = serde_json::from_slice(&body).unwrap();
    assert_eq!(history.order_id, REAL_ORDER);
    assert_eq!(history.count, 26);
    assert_eq!(history.dispute_id.as_deref(), Some(REAL_DISPUTE));
    assert_eq!(history.messages[0].action.as_deref(), Some("new-order"));
    assert_eq!(history.messages[0].role, "daemon");

    // 5. Ensure nsec or secrets are NEVER in response
    let json_text = String::from_utf8(body).unwrap();
    assert!(!json_text.contains("nsec"));
    assert!(!json_text.contains("secret"));
    assert!(!json_text.contains("preimage"));
    assert!(!json_text.contains("d911ec67fb78a65c"));
}

#[tokio::test]
async fn test_http_disputes_endpoint_links_public_disputes_to_orders() {
    let dir = tempfile::tempdir().unwrap();
    let node = fixture_keys(1);
    let mut cache = ChatCache::new();
    replay_real_traffic(&node, &mut cache);

    let mut orders = OrdersCache::new();
    let dispute = |id: &str, status: &str, updated_at: u64| DisputeSummary {
        id: id.into(),
        event_id: "e".repeat(64),
        status: status.into(),
        initiator: Some("buyer".into()),
        published_at: Some(1_791_063_450),
        updated_at,
    };
    assert_eq!(
        orders.try_insert_dispute(dispute(REAL_DISPUTE, "initiated", 10), 0),
        Some(None)
    );
    let unknown = "55555555-6666-7777-8888-999999999999";
    assert_eq!(
        orders.try_insert_dispute(dispute(unknown, "settled", 5), 0),
        Some(None)
    );
    let app = test_app(dir.path(), orders, cache);

    let (status, _) = get_json(&app, "/api/disputes", false).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, body) = get_json(&app, "/api/disputes", true).await;
    assert_eq!(status, StatusCode::OK);
    let disputes: Vec<chat::DisputeView> = serde_json::from_slice(&body).unwrap();
    assert_eq!(disputes.len(), 2);
    // Newest first. The order is known only for the dispute the daemon's own
    // messages mentioned; the public event never carries it.
    assert_eq!(disputes[0].dispute.id, REAL_DISPUTE);
    assert_eq!(disputes[0].order_id.as_deref(), Some(REAL_ORDER));
    assert!(disputes[0].is_open);
    assert_eq!(disputes[1].dispute.id, unknown);
    assert_eq!(disputes[1].order_id, None);
    assert!(!disputes[1].is_open);
    let flat: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(flat[0]["status"], "initiated");
    assert_eq!(flat[0]["initiator"], "buyer");
}

#[tokio::test]
async fn test_chat_worker_with_mock_relay() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let comm_keys = fixture_keys(1);
    let trader_keys = fixture_keys(2);

    identity::import(
        root,
        &comm_keys.secret_key().to_bech32().unwrap(),
        &comm_keys.public_key().to_bech32().unwrap(),
    )
    .unwrap();

    let (relay_url, mut req_rx, resp_tx, relay_handle) = spawn_mock_relay().await;
    let chat_cache: chat::SharedChatCache = Arc::new(RwLock::new(ChatCache::new()));

    let config = valid_config(vec![relay_url]);
    let (config_tx, config_rx) = watch::channel(MonitorCommand {
        config,
        npub: Some(comm_keys.public_key().to_bech32().unwrap()),
    });

    let timing = MonitorTiming::test_timing();
    let worker_handle = tokio::spawn(chat::chat_worker(
        chat_cache.clone(),
        root.to_path_buf(),
        config_rx,
        timing,
    ));

    // The worker subscribes to protocol v2 only: kind 14 to and from the node,
    // bounded in time and size.
    let node_hex = comm_keys.public_key().to_hex();
    for (sub, selector) in [("mostro_chat_recv", "#p"), ("mostro_chat_sent", "authors")] {
        let req = req_rx.recv().await.expect("subscription");
        let parsed: serde_json::Value = serde_json::from_str(&req).unwrap();
        assert_eq!(parsed[0], "REQ");
        assert_eq!(parsed[1], sub);
        assert_eq!(parsed[2]["kinds"], serde_json::json!([14]));
        assert_eq!(parsed[2][selector], serde_json::json!([node_hex]));
        assert!(parsed[2]["since"].as_u64().is_some());
        assert_eq!(parsed[2]["limit"], 2000);
    }

    let order_id = "88888888-9999-aaaa-bbbb-cccccccccccc";
    let reply = build_test_kind14_event(
        &comm_keys,
        &trader_keys.public_key(),
        &order_message("dispute-initiated-by-you", order_id),
        None,
    );
    resp_tx
        .send(format!(
            r#"["EVENT", "mostro_chat_sent", {}]"#,
            serde_json::to_string(&reply).unwrap()
        ))
        .await
        .unwrap();

    // Give worker time to process and decrypt
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    {
        let r = chat_cache.read().await;
        let history = r.to_history(order_id);
        assert_eq!(
            history.count, 1,
            "El mensaje del daemon debe ser recibido y descifrado por el chat_worker"
        );
        assert_eq!(history.messages[0].id, reply.id.to_hex());
        assert_eq!(
            history.messages[0].action.as_deref(),
            Some("dispute-initiated-by-you")
        );
    }

    drop(config_tx);
    worker_handle.abort();
    relay_handle.abort();
}
