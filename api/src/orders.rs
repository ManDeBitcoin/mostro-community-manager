use axum::{Json, extract::State};
use futures_util::{SinkExt, StreamExt};
use nostr::{Event, FromBech32, PublicKey, Timestamp};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, watch};
use tokio::task::JoinHandle;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

use crate::{AppState, chat::shorten, config::Configuration};

pub const JS_MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
pub const MAX_WS_FRAME_SIZE: usize = 128 * 1024; // 128 KB limit per message

/// Public Nostr event kinds published by mostrod (mostro-core `prelude`).
pub const ORDER_EVENT_KIND: u16 = 38383;
pub const INFO_EVENT_KIND: u16 = 38385;
pub const DISPUTE_EVENT_KIND: u16 = 38386;

/// Upper bounds for the in-memory caches. Closed orders are evicted first.
pub const MAX_CACHED_ORDERS: usize = 1000;
pub const MAX_CACHED_TOMBSTONES: usize = 5000;
pub const MAX_CACHED_DISPUTES: usize = 500;

/// The node info event is republished every `publish_mostro_info_interval`
/// (300 s in the pinned template). Two missed publications plus slack means
/// the daemon is no longer announcing itself.
pub const NODE_INFO_FRESH_SECS: u64 = 660;

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum MonitorState {
    Unconfigured,
    Connecting,
    Syncing,
    Live,
    Degraded,
    Disconnected,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct RelayStatus {
    pub url: String,
    pub state: MonitorState,
    pub last_error: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct OrderSummary {
    pub id: String, // uuid inside 'd' tag
    pub event_id: String,
    pub kind: String, // buy/sell
    pub status: String,
    pub fiat_code: String,
    pub fiat_amount_range: Vec<String>,
    pub amount_sats: u64,
    pub amount_sats_str: String, // Safe string representation for JavaScript clients
    pub payment_methods: Vec<String>,
    pub premium: i64,
    pub created_at: u64,
    pub expires_at: Option<u64>,
    /// Order creation time announced by the daemon (`published_at` tag). Unlike
    /// `created_at`, it does not change when the addressable event is revised.
    #[serde(default)]
    pub published_at: Option<u64>,
}

/// Public state of a dispute as announced by the daemon in kind 38386.
/// The event deliberately carries the dispute id only, never the order id.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct DisputeSummary {
    pub id: String,
    pub event_id: String,
    pub status: String,
    pub initiator: Option<String>,
    pub published_at: Option<u64>,
    pub updated_at: u64,
}

/// What the daemon itself announces in its kind 38385 instance-info event.
/// This is the only version/limits source that proves a daemon is alive.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct NodeInfo {
    pub event_id: String,
    pub created_at: u64,
    pub name: Option<String>,
    pub mostro_version: Option<String>,
    pub protocol_version: Option<String>,
    /// Every tag of the event (first value; `y` keeps the instance name).
    pub tags: BTreeMap<String, String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct OrdersSnapshot {
    pub state: MonitorState,
    pub is_stale: bool,
    pub last_update: u64,
    pub source_npub: Option<String>,
    pub relays: Vec<RelayStatus>,
    pub orders: Vec<OrderSummary>,
    #[serde(default)]
    pub disputes: Vec<DisputeSummary>,
    #[serde(default)]
    pub node_info: Option<NodeInfo>,
    /// Seconds since the daemon last announced itself, when known.
    #[serde(default)]
    pub node_info_age_secs: Option<u64>,
}

pub struct OrdersCache {
    pub state: MonitorState,
    pub is_stale: bool,
    pub last_update: u64,
    pub npub: Option<String>,
    pub relay_statuses: HashMap<String, RelayStatus>,
    pub events: HashMap<String, Event>,            // keyed by uuid
    pub closed_orders: HashMap<String, u64>,       // keyed by uuid -> closed_created_at
    pub disputes: HashMap<String, DisputeSummary>, // keyed by dispute uuid
    pub node_info: Option<NodeInfo>,
    pub generation: u64,
}

impl Default for OrdersCache {
    fn default() -> Self {
        Self::new()
    }
}

impl OrdersCache {
    pub fn new() -> Self {
        Self {
            state: MonitorState::Unconfigured,
            is_stale: false,
            last_update: 0,
            npub: None,
            relay_statuses: HashMap::new(),
            events: HashMap::new(),
            closed_orders: HashMap::new(),
            disputes: HashMap::new(),
            node_info: None,
            generation: 0,
        }
    }

    pub fn to_snapshot(&self) -> OrdersSnapshot {
        let now_secs = Timestamp::now().as_secs();
        let mut orders: Vec<OrderSummary> = self
            .events
            .values()
            .filter_map(|e| parse_order_to_summary(e, now_secs))
            .collect();
        // Sort newest first
        orders.sort_by_key(|order| std::cmp::Reverse(order.created_at));

        let mut relays: Vec<RelayStatus> = self.relay_statuses.values().cloned().collect();
        relays.sort_by(|a, b| a.url.cmp(&b.url));

        let mut disputes: Vec<DisputeSummary> = self.disputes.values().cloned().collect();
        disputes.sort_by_key(|dispute| std::cmp::Reverse(dispute.updated_at));

        OrdersSnapshot {
            state: self.state,
            is_stale: self.is_stale,
            last_update: self.last_update,
            source_npub: self.npub.clone(),
            relays,
            orders,
            disputes,
            node_info_age_secs: self.node_info_age_secs(now_secs),
            node_info: self.node_info.clone(),
        }
    }

    /// Age of the last info event the daemon published, in seconds.
    pub fn node_info_age_secs(&self, now_secs: u64) -> Option<u64> {
        self.node_info
            .as_ref()
            .map(|info| now_secs.saturating_sub(info.created_at))
    }

    /// Keeps the newest kind 38385 event. Returns true when it replaced the cache.
    pub fn try_set_node_info(&mut self, info: NodeInfo, gen_id: u64) -> bool {
        if gen_id != self.generation {
            return false;
        }
        if self
            .node_info
            .as_ref()
            .is_some_and(|current| current.created_at >= info.created_at)
        {
            return false;
        }
        self.node_info = Some(info);
        self.last_update = Timestamp::now().as_secs();
        true
    }

    /// Stores the newest revision of a dispute. Returns the previous status
    /// (`None` when the dispute was unknown) if the revision was accepted.
    pub fn try_insert_dispute(
        &mut self,
        dispute: DisputeSummary,
        gen_id: u64,
    ) -> Option<Option<String>> {
        if gen_id != self.generation {
            return None;
        }
        let previous = match self.disputes.get(&dispute.id) {
            Some(existing) if existing.updated_at >= dispute.updated_at => return None,
            Some(existing) => Some(existing.status.clone()),
            None => None,
        };
        if previous.is_none() && self.disputes.len() >= MAX_CACHED_DISPUTES {
            // Evict the stalest revision so a long-lived node keeps seeing new disputes.
            if let Some(oldest) = self
                .disputes
                .values()
                .min_by_key(|d| d.updated_at)
                .map(|d| d.id.clone())
            {
                self.disputes.remove(&oldest);
            }
        }
        self.disputes.insert(dispute.id.clone(), dispute);
        self.last_update = Timestamp::now().as_secs();
        Some(previous)
    }

    pub fn clear_for_new_config(&mut self, npub: Option<String>, relays: &[String], gen_id: u64) {
        self.generation = gen_id;
        self.npub = npub.clone();
        self.events.clear();
        self.closed_orders.clear();
        self.disputes.clear();
        self.node_info = None;
        self.relay_statuses.clear();
        self.last_update = Timestamp::now().as_secs();

        if npub.is_none() || relays.is_empty() {
            self.state = MonitorState::Unconfigured;
            self.is_stale = false;
        } else {
            for r in relays {
                self.relay_statuses.insert(
                    r.clone(),
                    RelayStatus {
                        url: r.clone(),
                        state: MonitorState::Connecting,
                        last_error: None,
                    },
                );
            }
            self.state = MonitorState::Connecting;
            self.is_stale = true;
        }
    }

    pub fn compute_aggregate_state(&mut self) {
        if self.npub.is_none() || self.relay_statuses.is_empty() {
            self.state = MonitorState::Unconfigured;
            self.is_stale = false;
            return;
        }

        let mut any_live = false;
        let mut any_syncing = false;
        let mut any_connecting = false;
        let mut any_disconnected = false;

        for r in self.relay_statuses.values() {
            match r.state {
                MonitorState::Live => any_live = true,
                MonitorState::Syncing => any_syncing = true,
                MonitorState::Connecting => any_connecting = true,
                MonitorState::Disconnected => any_disconnected = true,
                MonitorState::Degraded => any_live = true,
                MonitorState::Unconfigured => {}
            }
        }

        if any_live {
            if any_disconnected {
                self.state = MonitorState::Degraded;
            } else {
                self.state = MonitorState::Live;
            }
            self.is_stale = false;
        } else if any_syncing {
            self.state = MonitorState::Syncing;
            self.is_stale = true;
        } else if any_connecting {
            self.state = MonitorState::Connecting;
            self.is_stale = true;
        } else {
            self.state = MonitorState::Disconnected;
            self.is_stale = true;
        }
    }

    pub fn update_relay_status(
        &mut self,
        url: &str,
        state: MonitorState,
        error: Option<String>,
        gen_id: u64,
    ) {
        if gen_id != self.generation {
            return;
        }
        if let Some(r) = self.relay_statuses.get_mut(url) {
            r.state = state;
            r.last_error = error;
        } else {
            self.relay_statuses.insert(
                url.to_string(),
                RelayStatus {
                    url: url.to_string(),
                    state,
                    last_error: error,
                },
            );
        }
        self.compute_aggregate_state();
    }

    pub fn try_insert_event(
        &mut self,
        event: Event,
        uuid: String,
        is_closed: bool,
        gen_id: u64,
    ) -> bool {
        if gen_id != self.generation {
            return false;
        }

        // Tombstone check: if order was previously closed, older replayed pending events are rejected
        if let Some(&closed_at) = self.closed_orders.get(&uuid)
            && !is_closed
            && event.created_at.as_secs() <= closed_at
        {
            return false;
        }

        // Replacement logic: higher created_at, or tie-break lexicographically smaller event.id
        let should_insert = if let Some(existing) = self.events.get(&uuid) {
            if event.created_at > existing.created_at {
                true
            } else if event.created_at == existing.created_at {
                event.id < existing.id
            } else {
                false
            }
        } else {
            true
        };

        if should_insert {
            if is_closed {
                if self.closed_orders.len() >= MAX_CACHED_TOMBSTONES
                    && !self.closed_orders.contains_key(&uuid)
                    && let Some(oldest) = self
                        .closed_orders
                        .iter()
                        .min_by_key(|(_, closed_at)| **closed_at)
                        .map(|(id, _)| id.clone())
                {
                    self.closed_orders.remove(&oldest);
                }
                self.closed_orders
                    .insert(uuid.clone(), event.created_at.as_secs());
            }

            if self.events.len() >= MAX_CACHED_ORDERS && !self.events.contains_key(&uuid) {
                self.evict_one_order();
            }
            if self.events.len() < MAX_CACHED_ORDERS || self.events.contains_key(&uuid) {
                self.events.insert(uuid, event);
                self.last_update = Timestamp::now().as_secs();
                return true;
            }
        }
        false
    }

    /// Makes room for a new order: the oldest closed order goes first. Open
    /// orders are never evicted, so a full book of open orders still rejects.
    fn evict_one_order(&mut self) {
        let victim = self
            .events
            .iter()
            .filter(|(id, _)| self.closed_orders.contains_key(*id))
            .min_by_key(|(_, event)| event.created_at)
            .map(|(id, _)| id.clone());
        if let Some(id) = victim {
            self.events.remove(&id);
        }
    }
}

pub type SharedOrders = Arc<RwLock<OrdersCache>>;

#[derive(Clone, Debug)]
pub struct MonitorTiming {
    pub connect_timeout: Duration,
    pub eose_timeout: Duration,
    pub ping_interval: Duration,
    pub ping_timeout: Duration,
    pub reconnect_initial: Duration,
    pub reconnect_max: Duration,
    pub max_future_drift_secs: u64,
}

impl Default for MonitorTiming {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            eose_timeout: Duration::from_secs(5),
            ping_interval: Duration::from_secs(15),
            ping_timeout: Duration::from_secs(5),
            reconnect_initial: Duration::from_millis(500),
            reconnect_max: Duration::from_secs(5),
            max_future_drift_secs: 60,
        }
    }
}

impl MonitorTiming {
    pub fn test_timing() -> Self {
        Self {
            connect_timeout: Duration::from_millis(300),
            eose_timeout: Duration::from_millis(300),
            ping_interval: Duration::from_millis(200),
            ping_timeout: Duration::from_millis(150),
            reconnect_initial: Duration::from_millis(30),
            reconnect_max: Duration::from_millis(100),
            max_future_drift_secs: 60,
        }
    }
}

pub fn is_valid_uuid(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (i, &b) in bytes.iter().enumerate() {
        if i == 8 || i == 13 || i == 18 || i == 23 {
            if b != b'-' {
                return false;
            }
        } else if !b.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

/// Statuses mostrod can publish in the `s` tag of kind 38383 (v0.19.x,
/// `create_status_tags` in upstream `src/nip33.rs`). v0.19.2 maps
/// `completed-by-admin` but never assigns it.
///
/// The tag is coarse. `in-progress` is published when a sell order waits for
/// the buyer's invoice or a buy order waits for the seller's payment. Later
/// internal states (`active`, `fiat-sent`, `dispute`) publish nothing, so the
/// last value stays; a sell order taken with the invoice attached never
/// leaves `pending` until it closes. Disputes are announced in kind 38386.
pub const PUBLIC_ORDER_STATUSES: [&str; 5] = [
    "pending",
    "in-progress",
    "success",
    "canceled",
    "completed-by-admin",
];

/// A status is accepted when it is well formed. The monitor must not freeze an
/// order on its last known revision because a newer daemon added a status.
pub fn is_valid_status(status: &str) -> bool {
    !status.is_empty()
        && status.len() <= 40
        && status
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// Terminal statuses. `completed-by-admin` and `canceled` are what a solver
/// resolution looks like on the wire; the rest are kept for older daemons.
pub fn is_closed_status(status: &str) -> bool {
    matches!(
        status,
        "canceled"
            | "success"
            | "completed-by-admin"
            | "canceled-by-admin"
            | "cooperatively-canceled"
            | "settled-by-admin"
            | "completed"
            | "failed"
            | "expired"
    )
}

/// What the panel keeps of the `pm` tag of an order: the daemon copies there,
/// split on commas, the text the maker typed.
pub const MAX_ORDER_PAYMENT_METHODS: usize = 10;
pub const MAX_PAYMENT_METHOD_CHARS: usize = 60;

/// A fiat amount is a whole number in mostro-core (`i64`). Fifteen digits is
/// more than any currency needs and still exact as a JavaScript number.
const MAX_FIAT_AMOUNT_DIGITS: usize = 15;

/// The payment methods of an order as the panel shows them: each one cleaned
/// and cut like the sender text of the chat timeline, and no more than
/// [`MAX_ORDER_PAYMENT_METHODS`]. A longer list ends in `…` so that the cut
/// shows.
fn clean_payment_methods(values: &[String]) -> Vec<String> {
    let mut methods = Vec::new();
    for value in values {
        let method = shorten(value.trim(), MAX_PAYMENT_METHOD_CHARS);
        let method = method.trim();
        if method.is_empty() {
            continue;
        }
        if methods.len() == MAX_ORDER_PAYMENT_METHODS {
            methods.push("…".to_string());
            break;
        }
        methods.push(method.to_string());
    }
    methods
}

/// The `fa` tag as mostrod writes it (`create_fiat_amt_array`): the fiat
/// amount, or the two ends of a range, in digits. Anything else is not an
/// amount the panel can show, and an empty list says so.
fn clean_fiat_amounts(values: &[String]) -> Vec<String> {
    let is_amount = |value: &String| {
        !value.is_empty()
            && value.len() <= MAX_FIAT_AMOUNT_DIGITS
            && value.bytes().all(|b| b.is_ascii_digit())
    };
    if matches!(values.len(), 1 | 2) && values.iter().all(is_amount) {
        values.to_vec()
    } else {
        Vec::new()
    }
}

/// Parses and strictly validates a Nostr Kind 38383 order event against upstream protocol rules.
pub fn parse_and_validate_order_event(
    event: &Event,
    expected_pubkey: &PublicKey,
    now_secs: u64,
    max_future_drift_secs: u64,
) -> Result<(OrderSummary, bool), &'static str> {
    event.verify().map_err(|_| "Firma criptográfica inválida")?;

    if event.pubkey != *expected_pubkey {
        return Err("Autor del evento no corresponde a la identidad esperada");
    }

    if event.kind.as_u16() != 38383 {
        return Err("Kind de evento no es 38383");
    }

    if event.created_at.as_secs() > now_secs.saturating_add(max_future_drift_secs) {
        return Err("Timestamp del evento en el futuro fuera de tolerancia");
    }

    let mut id = None;
    let mut kind = None;
    let mut status = None;
    let mut fiat_code = None;
    let mut fiat_amount_range = Vec::new();
    let mut amount_sats: u64 = 0;
    let mut payment_methods = Vec::new();
    let mut premium: i64 = 0;
    let mut expires_at = None;
    let mut expiration_nip40 = None;
    let mut published_at = None;
    let mut has_y_mostro = false;
    let mut has_z_order = false;

    for tag in event.tags.iter() {
        let s = tag.as_slice();
        if s.len() >= 2 {
            match s[0].as_str() {
                "y" if s[1] == "mostro" => has_y_mostro = true,
                "z" if s[1] == "order" => has_z_order = true,
                "d" => {
                    let uuid_candidate = &s[1];
                    if is_valid_uuid(uuid_candidate) {
                        id = Some(uuid_candidate.clone());
                    } else {
                        return Err("Tag d no contiene un UUID válido");
                    }
                }
                "k" => {
                    let k = s[1].to_lowercase();
                    if k == "buy" || k == "sell" {
                        kind = Some(k);
                    } else {
                        return Err("Tag k no es 'buy' ni 'sell'");
                    }
                }
                "s" => {
                    let st = s[1].to_lowercase();
                    if is_valid_status(&st) {
                        status = Some(st);
                    } else {
                        return Err("Tag s no contiene un estado de orden válido");
                    }
                }
                "f" => {
                    let f = s[1].to_uppercase();
                    if (3..=5).contains(&f.len()) && f.bytes().all(|b| b.is_ascii_uppercase()) {
                        fiat_code = Some(f);
                    }
                }
                "fa" => {
                    fiat_amount_range = clean_fiat_amounts(&s[1..]);
                }
                "amt" => {
                    if let Ok(parsed) = s[1].parse::<u64>() {
                        if parsed <= JS_MAX_SAFE_INTEGER {
                            amount_sats = parsed;
                        } else {
                            return Err("Monto de satoshis excede el límite seguro de precisión");
                        }
                    }
                }
                "pm" => {
                    payment_methods = clean_payment_methods(&s[1..]);
                }
                "premium" => {
                    if let Ok(p) = s[1].parse::<i64>() {
                        premium = p;
                    }
                }
                "expires_at" => {
                    expires_at = s[1].parse::<u64>().ok();
                }
                "expiration" => {
                    expiration_nip40 = s[1].parse::<u64>().ok();
                }
                "published_at" => {
                    published_at = s[1].parse::<u64>().ok();
                }
                _ => {}
            }
        }
    }

    if !has_y_mostro || !has_z_order {
        return Err("Faltan tags obligatorios y=mostro o z=order");
    }

    let id = id.ok_or("Falta tag d con UUID")?;
    let kind = kind.ok_or("Falta tag k (buy/sell)")?;
    let status = status.ok_or("Falta tag s con estado")?;
    let fiat_code = fiat_code.ok_or("Falta tag f con un código de moneda válido")?;

    // NIP-40 expiration check
    if let Some(exp) = expiration_nip40
        && exp <= now_secs
    {
        return Err("Evento expirado según NIP-40");
    }

    // Mostro internal expires_at check: pending offers expired in the past are not active
    if status == "pending"
        && let Some(exp) = expires_at
        && exp <= now_secs
    {
        return Err("Oferta expirada según expires_at");
    }

    let is_closed = is_closed_status(&status);

    Ok((
        OrderSummary {
            id,
            event_id: event.id.to_hex(),
            kind,
            status,
            fiat_code,
            fiat_amount_range,
            amount_sats,
            amount_sats_str: amount_sats.to_string(),
            payment_methods,
            premium,
            created_at: event.created_at.as_secs(),
            expires_at,
            published_at,
        },
        is_closed,
    ))
}

/// Parses a kind 38386 dispute event signed by the node.
pub fn parse_dispute_event(
    event: &Event,
    expected_pubkey: &PublicKey,
    now_secs: u64,
    max_future_drift_secs: u64,
) -> Result<DisputeSummary, &'static str> {
    event.verify().map_err(|_| "Firma criptográfica inválida")?;
    if event.pubkey != *expected_pubkey {
        return Err("Autor del evento no corresponde a la identidad esperada");
    }
    if event.kind.as_u16() != DISPUTE_EVENT_KIND {
        return Err("Kind de evento no es 38386");
    }
    if event.created_at.as_secs() > now_secs.saturating_add(max_future_drift_secs) {
        return Err("Timestamp del evento en el futuro fuera de tolerancia");
    }

    let mut id = None;
    let mut status = None;
    let mut initiator = None;
    let mut published_at = None;
    let mut has_y_mostro = false;
    let mut has_z_dispute = false;
    for tag in event.tags.iter() {
        let s = tag.as_slice();
        if s.len() < 2 {
            continue;
        }
        match s[0].as_str() {
            "y" if s[1] == "mostro" => has_y_mostro = true,
            "z" if s[1] == "dispute" => has_z_dispute = true,
            "d" if is_valid_uuid(&s[1]) => id = Some(s[1].clone()),
            "s" => {
                let st = s[1].to_lowercase();
                if is_valid_status(&st) {
                    status = Some(st);
                }
            }
            "initiator" if matches!(s[1].as_str(), "buyer" | "seller") => {
                initiator = Some(s[1].clone());
            }
            "published_at" => published_at = s[1].parse::<u64>().ok(),
            "expiration" if s[1].parse::<u64>().is_ok_and(|exp| exp <= now_secs) => {
                return Err("Evento expirado según NIP-40");
            }
            _ => {}
        }
    }
    if !has_y_mostro || !has_z_dispute {
        return Err("Faltan tags obligatorios y=mostro o z=dispute");
    }
    Ok(DisputeSummary {
        id: id.ok_or("Falta tag d con UUID de disputa")?,
        event_id: event.id.to_hex(),
        status: status.ok_or("Falta tag s con estado de disputa")?,
        initiator,
        published_at,
        updated_at: event.created_at.as_secs(),
    })
}

/// A dispute is open until a solver resolves it (`settled`, `seller-refunded`)
/// or the parties close it themselves (`released`).
pub fn is_open_dispute_status(status: &str) -> bool {
    matches!(status, "initiated" | "in-progress")
}

/// Parses the kind 38385 instance-info event signed by the node.
pub fn parse_node_info_event(
    event: &Event,
    expected_pubkey: &PublicKey,
    now_secs: u64,
    max_future_drift_secs: u64,
) -> Result<NodeInfo, &'static str> {
    event.verify().map_err(|_| "Firma criptográfica inválida")?;
    if event.pubkey != *expected_pubkey {
        return Err("Autor del evento no corresponde a la identidad esperada");
    }
    if event.kind.as_u16() != INFO_EVENT_KIND {
        return Err("Kind de evento no es 38385");
    }
    if event.created_at.as_secs() > now_secs.saturating_add(max_future_drift_secs) {
        return Err("Timestamp del evento en el futuro fuera de tolerancia");
    }

    let mut tags = BTreeMap::new();
    let mut name = None;
    let mut is_info = false;
    for tag in event.tags.iter() {
        let s = tag.as_slice();
        if s.len() < 2 || s[0].len() > 64 || s[1].len() > 512 || tags.len() >= 64 {
            continue;
        }
        match s[0].as_str() {
            "z" if s[1] == "info" => is_info = true,
            "y" if s[1] == "mostro" => name = s.get(2).filter(|n| n.len() <= 256).cloned(),
            _ => {}
        }
        tags.entry(s[0].clone()).or_insert_with(|| s[1].clone());
    }
    if !is_info {
        return Err("Falta tag obligatorio z=info");
    }
    Ok(NodeInfo {
        event_id: event.id.to_hex(),
        created_at: event.created_at.as_secs(),
        name,
        mostro_version: tags.get("mostro_version").cloned(),
        protocol_version: tags.get("protocol_version").cloned(),
        tags,
    })
}

fn parse_order_to_summary(event: &Event, now_secs: u64) -> Option<OrderSummary> {
    let mut id: Option<String> = None;
    let mut kind: Option<String> = None;
    let mut status: Option<String> = None;
    let mut fiat_code: Option<String> = None;
    let mut fiat_amount_range = Vec::new();
    let mut amount_sats: u64 = 0;
    let mut payment_methods = Vec::new();
    let mut premium: i64 = 0;
    let mut expires_at: Option<u64> = None;
    let mut expiration_nip40: Option<u64> = None;
    let mut published_at: Option<u64> = None;

    for tag in event.tags.iter() {
        let s = tag.as_slice();
        if s.len() >= 2 {
            match s[0].as_str() {
                "d" => id = Some(s[1].clone()),
                "k" => kind = Some(s[1].clone()),
                "s" => status = Some(s[1].clone()),
                "f" => fiat_code = Some(s[1].clone()),
                "fa" => fiat_amount_range = clean_fiat_amounts(&s[1..]),
                "amt" => amount_sats = s[1].parse().unwrap_or(0),
                "pm" => payment_methods = clean_payment_methods(&s[1..]),
                "premium" => premium = s[1].parse().unwrap_or(0),
                "expires_at" => expires_at = s[1].parse().ok(),
                "expiration" => expiration_nip40 = s[1].parse().ok(),
                "published_at" => published_at = s[1].parse().ok(),
                _ => {}
            }
        }
    }

    let status = status?;
    if let Some(exp) = expiration_nip40
        && exp <= now_secs
    {
        return None;
    }
    if status == "pending"
        && let Some(exp) = expires_at
        && exp <= now_secs
    {
        return None;
    }

    Some(OrderSummary {
        id: id?,
        event_id: event.id.to_hex(),
        kind: kind?,
        status,
        fiat_code: fiat_code?,
        fiat_amount_range,
        amount_sats,
        amount_sats_str: amount_sats.to_string(),
        payment_methods,
        premium,
        created_at: event.created_at.as_secs(),
        expires_at,
        published_at,
    })
}

#[derive(Clone, Debug)]
pub struct MonitorCommand {
    pub config: Configuration,
    pub npub: Option<String>,
}

async fn run_relay_worker(
    relay_url: String,
    pubkey: PublicKey,
    cache: SharedOrders,
    generation: u64,
    timing: MonitorTiming,
    notifications: Option<Arc<crate::notifications::NotificationHub>>,
) {
    let mut reconnect_delay = timing.reconnect_initial;
    let mut notified_down = false;

    loop {
        // The relay starts as "connecting" when the configuration is loaded. A
        // retry keeps the last result: reporting "connecting" again on every
        // attempt would hide an outage for as long as each attempt takes.
        let connect_fut = connect_async(&relay_url);
        let ws_res = tokio::time::timeout(timing.connect_timeout, connect_fut).await;

        let mut ws_stream = match ws_res {
            Ok(Ok((ws, _))) => {
                notified_down = false;
                ws
            }
            Ok(Err(e)) => {
                {
                    let mut w = cache.write().await;
                    w.update_relay_status(
                        &relay_url,
                        MonitorState::Disconnected,
                        Some(format!("Fallo de conexión: {e}")),
                        generation,
                    );
                }
                if let Some(ref hub) = notifications
                    && !notified_down
                {
                    hub.publish(crate::notifications::Notification::relay_alert(
                        "Relay desconectado",
                        &format!("Fallo de conexión con {relay_url}: {e}"),
                        "warning",
                        Some(serde_json::json!({"relay": relay_url})),
                    ))
                    .await;
                    notified_down = true;
                }
                tokio::time::sleep(reconnect_delay).await;
                reconnect_delay = (reconnect_delay * 2).min(timing.reconnect_max);
                continue;
            }
            Err(_) => {
                {
                    let mut w = cache.write().await;
                    w.update_relay_status(
                        &relay_url,
                        MonitorState::Disconnected,
                        Some("Timeout en conexión a relay".into()),
                        generation,
                    );
                }
                if let Some(ref hub) = notifications
                    && !notified_down
                {
                    hub.publish(crate::notifications::Notification::relay_alert(
                        "Relay desconectado",
                        &format!("Timeout en conexión con {relay_url}"),
                        "warning",
                        Some(serde_json::json!({"relay": relay_url})),
                    ))
                    .await;
                    notified_down = true;
                }
                tokio::time::sleep(reconnect_delay).await;
                reconnect_delay = (reconnect_delay * 2).min(timing.reconnect_max);
                continue;
            }
        };

        // Reset reconnect delay on successful connection
        reconnect_delay = timing.reconnect_initial;

        {
            let mut w = cache.write().await;
            w.update_relay_status(&relay_url, MonitorState::Syncing, None, generation);
        }

        // One subscription for everything the node publishes about itself:
        // orders (38383), its instance info (38385) and disputes (38386).
        let req_msg = format!(
            "[\"REQ\", \"mostro_monitor\", {{\"kinds\": [{ORDER_EVENT_KIND}, {INFO_EVENT_KIND}, {DISPUTE_EVENT_KIND}], \"authors\": [\"{}\"]}}]",
            pubkey.to_hex()
        );

        if ws_stream.send(Message::Text(req_msg.into())).await.is_err() {
            let mut w = cache.write().await;
            w.update_relay_status(
                &relay_url,
                MonitorState::Disconnected,
                Some("Fallo al enviar suscripción REQ".into()),
                generation,
            );
            tokio::time::sleep(reconnect_delay).await;
            continue;
        }

        let mut ping_interval = tokio::time::interval(timing.ping_interval);
        ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        let mut eose_received = false;
        let mut eose_deadline = Box::pin(tokio::time::sleep(timing.eose_timeout));

        loop {
            tokio::select! {
                _ = ping_interval.tick() => {
                    let ping_payload = vec![1, 2, 3, 4];
                    if ws_stream.send(Message::Ping(ping_payload.into())).await.is_err() {
                        break;
                    }
                }
                _ = &mut eose_deadline, if !eose_received => {
                    // EOSE timed out; mark live to avoid hanging sync forever
                    eose_received = true;
                    let mut w = cache.write().await;
                    w.update_relay_status(&relay_url, MonitorState::Live, None, generation);
                }
                msg_opt = ws_stream.next() => {
                    match msg_opt {
                        Some(Ok(Message::Text(text))) => {
                            if text.len() > MAX_WS_FRAME_SIZE {
                                continue;
                            }
                            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text)
                                && let Some(arr) = val.as_array()
                            {
                                if arr.len() >= 3 && arr[0] == "EVENT" && arr[1] == "mostro_monitor" {
                                    if let Ok(event) = serde_json::from_value::<Event>(arr[2].clone()) {
                                        let now = Timestamp::now().as_secs();
                                        let drift = timing.max_future_drift_secs;
                                        match event.kind.as_u16() {
                                            ORDER_EVENT_KIND => {
                                                if let Ok((summary, is_closed)) =
                                                    parse_and_validate_order_event(&event, &pubkey, now, drift)
                                                {
                                                    let mut w = cache.write().await;
                                                    w.try_insert_event(event, summary.id, is_closed, generation);
                                                }
                                            }
                                            INFO_EVENT_KIND => {
                                                if let Ok(info) = parse_node_info_event(&event, &pubkey, now, drift) {
                                                    let mut w = cache.write().await;
                                                    w.try_set_node_info(info, generation);
                                                }
                                            }
                                            DISPUTE_EVENT_KIND => {
                                                if let Ok(dispute) = parse_dispute_event(&event, &pubkey, now, drift) {
                                                    let dispute_id = dispute.id.clone();
                                                    let initiator = dispute.initiator.clone();
                                                    let is_open = is_open_dispute_status(&dispute.status);
                                                    // Replays of old disputes at connect time must not alert again.
                                                    let is_recent = eose_received
                                                        || now.saturating_sub(dispute.updated_at) <= 3600;
                                                    let inserted = {
                                                        let mut w = cache.write().await;
                                                        w.try_insert_dispute(dispute, generation)
                                                    };
                                                    if inserted == Some(None)
                                                        && is_open
                                                        && is_recent
                                                        && let Some(ref hub) = notifications
                                                    {
                                                        let opened_by = match initiator.as_deref() {
                                                            Some("buyer") => ", abierta por el comprador",
                                                            Some("seller") => ", abierta por el vendedor",
                                                            _ => "",
                                                        };
                                                        hub.publish(crate::notifications::Notification::dispute_alert(
                                                            &format!("El nodo anuncia la disputa {dispute_id}{opened_by}. Necesita un mediador."),
                                                            Some(serde_json::json!({
                                                                "dispute_id": dispute_id,
                                                                "initiator": initiator,
                                                            })),
                                                        )).await;
                                                    }
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                } else if arr.len() >= 2 && arr[0] == "EOSE" && arr[1] == "mostro_monitor" {
                                    eose_received = true;
                                    let mut w = cache.write().await;
                                    w.update_relay_status(&relay_url, MonitorState::Live, None, generation);
                                }
                            }
                        }
                        Some(Ok(Message::Ping(p))) => {
                            let _ = ws_stream.send(Message::Pong(p)).await;
                        }
                        Some(Ok(Message::Pong(_))) => {}
                        Some(Ok(Message::Close(_))) | None => {
                            break;
                        }
                        Some(Err(_)) => {
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }

        // Clean close attempt
        let _ = ws_stream
            .send(Message::Text("[\"CLOSE\", \"mostro_monitor\"]".into()))
            .await;
        let _ = ws_stream.close(None).await;

        {
            let mut w = cache.write().await;
            w.update_relay_status(
                &relay_url,
                MonitorState::Disconnected,
                Some("Conexión finalizada o interrumpida".into()),
                generation,
            );
        }

        tokio::time::sleep(reconnect_delay).await;
        reconnect_delay = (reconnect_delay * 2).min(timing.reconnect_max);
    }
}

pub async fn monitor_worker(
    cache: SharedOrders,
    config_rx: watch::Receiver<MonitorCommand>,
    timing: MonitorTiming,
) {
    monitor_worker_with_notifications(cache, config_rx, timing, None).await;
}

pub async fn monitor_worker_with_notifications(
    cache: SharedOrders,
    mut config_rx: watch::Receiver<MonitorCommand>,
    timing: MonitorTiming,
    notifications: Option<Arc<crate::notifications::NotificationHub>>,
) {
    let mut current_generation: u64 = 0;
    let mut relay_handles: Vec<JoinHandle<()>> = Vec::new();
    let mut last_npub: Option<String> = None;
    let mut last_relays: Vec<String> = Vec::new();

    loop {
        let cmd = config_rx.borrow_and_update().clone();
        let npub_changed = cmd.npub != last_npub;
        let relays_changed = cmd.config.nostr.relays != last_relays;

        if npub_changed || relays_changed || current_generation == 0 {
            for h in relay_handles.drain(..) {
                h.abort();
            }

            current_generation = current_generation.wrapping_add(1);
            last_npub = cmd.npub.clone();
            last_relays = cmd.config.nostr.relays.clone();

            {
                let mut w = cache.write().await;
                w.clear_for_new_config(
                    cmd.npub.clone(),
                    &cmd.config.nostr.relays,
                    current_generation,
                );
            }

            if let (Some(npub_str), false) = (&cmd.npub, cmd.config.nostr.relays.is_empty())
                && let Ok(pubkey) = PublicKey::from_bech32(npub_str)
            {
                for relay_url in cmd.config.nostr.relays.clone() {
                    let h = tokio::spawn(run_relay_worker(
                        relay_url,
                        pubkey,
                        cache.clone(),
                        current_generation,
                        timing.clone(),
                        notifications.clone(),
                    ));
                    relay_handles.push(h);
                }
            }
        }

        if config_rx.changed().await.is_err() {
            break;
        }
    }

    for h in relay_handles {
        h.abort();
    }
}

pub async fn get_orders_handler(State(state): State<AppState>) -> Json<OrdersSnapshot> {
    let r = state.orders.read().await;
    Json(r.to_snapshot())
}
