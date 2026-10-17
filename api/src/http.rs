use crate::{
    adapters::Integrations,
    auth::{self, AuthMode, LoginError, PanelAuth},
    backup::{self, AutoBackupState},
    card_publication::{CardPublication, PublicationView},
    chat::{self, SharedChatCache},
    config::Configuration,
    connection, daemon, identity,
    notifications::{Notification, NotificationHub},
    orders::{self, SharedOrders},
    simulation,
    store::{Document, Store},
};
use axum::{
    Json, Router,
    extract::{ConnectInfo, DefaultBodyLimit, Extension, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post, put},
};
use futures_util::stream::{self, Stream};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    convert::Infallible,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub type Error = (StatusCode, Json<Value>);

pub fn error(status: StatusCode, message: &str) -> Error {
    (status, Json(json!({"error": message})))
}

struct ClientBucket {
    tokens: f64,
    last_update: Instant,
}

pub struct RateLimiter {
    buckets: Mutex<HashMap<String, ClientBucket>>,
    pub max_tokens: f64,
    pub refill_rate: f64,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(120.0, 20.0) // 120 burst capacity, 20 tokens per second
    }
}

impl RateLimiter {
    pub fn new(max_tokens: f64, refill_rate: f64) -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
            max_tokens,
            refill_rate,
        }
    }

    pub fn check(&self, key: &str, cost: f64) -> bool {
        let mut buckets = match self.buckets.lock() {
            Ok(b) => b,
            Err(_) => return true,
        };

        let now = Instant::now();
        if buckets.len() > 1000 {
            buckets.retain(|_, b| now.duration_since(b.last_update).as_secs() < 300);
        }

        let bucket = buckets
            .entry(key.to_string())
            .or_insert_with(|| ClientBucket {
                tokens: self.max_tokens,
                last_update: now,
            });

        let elapsed = now.duration_since(bucket.last_update).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * self.refill_rate).min(self.max_tokens);
        bucket.last_update = now;

        if bucket.tokens >= cost {
            bucket.tokens -= cost;
            true
        } else {
            false
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Mutex<Store>>,
    pub integrations: Integrations,
    pub orders: SharedOrders,
    pub chat: SharedChatCache,
    pub monitor_tx: tokio::sync::watch::Sender<orders::MonitorCommand>,
    pub notifications: Arc<NotificationHub>,
    pub backup_state: Arc<tokio::sync::RwLock<AutoBackupState>>,
    pub rate_limiter: Arc<RateLimiter>,
    /// The worker that publishes the community card runs apart
    /// (`card_publication::CardPublisher`); this is how it is told to look again.
    pub card_publication: CardPublication,
    pub auth: Arc<PanelAuth>,
}

impl AppState {
    pub fn new(
        store: Arc<Mutex<Store>>,
        integrations: Integrations,
        orders: SharedOrders,
        chat: SharedChatCache,
        monitor_tx: tokio::sync::watch::Sender<orders::MonitorCommand>,
    ) -> Self {
        Self {
            store,
            integrations,
            orders,
            chat,
            monitor_tx,
            notifications: Arc::new(NotificationHub::default()),
            backup_state: Arc::new(tokio::sync::RwLock::new(AutoBackupState::default())),
            rate_limiter: Arc::new(RateLimiter::default()),
            card_publication: CardPublication::default(),
            auth: Arc::new(PanelAuth::open()),
        }
    }
}

pub fn router(state: AppState) -> Router {
    let api_routes = Router::new()
        .route("/api/health", get(health))
        .route("/api/auth/session", get(auth_session_handler))
        .route("/api/auth/login", post(login_handler))
        .route("/api/auth/logout", post(logout_handler))
        .route("/api/dashboard", get(dashboard))
        .route("/api/community", get(community).merge(put(save_community)))
        .route("/api/community/card", get(community_card_handler))
        .route(
            "/api/community/card/publication",
            get(card_publication_handler).put(card_publication_switch_handler),
        )
        .route("/api/community/presets", get(community_presets_handler))
        .route("/api/identity/generate", post(identity_generate_handler))
        .route("/api/identity/import", post(identity_import_handler))
        .route("/api/connection", get(connection_info_handler))
        .route("/api/daemon/status", get(daemon_status_handler))
        .route(
            "/api/daemon/activate",
            put(daemon_activate_handler).post(daemon_activate_handler),
        )
        .route(
            "/api/daemon/start",
            put(daemon_activate_handler).post(daemon_activate_handler),
        )
        .route(
            "/api/daemon/deactivate",
            put(daemon_deactivate_handler).post(daemon_deactivate_handler),
        )
        .route(
            "/api/daemon/stop",
            put(daemon_deactivate_handler).post(daemon_deactivate_handler),
        )
        .route(
            "/api/simulation/scenarios",
            get(simulation_scenarios_handler),
        )
        .route("/api/simulation/run", post(simulation_run_handler))
        .route("/api/orders", get(orders::get_orders_handler))
        .route("/api/chat/{order_id}", get(chat::get_chat_handler))
        .route("/api/disputes", get(chat::get_disputes_handler))
        .route("/api/notifications", get(notifications_list_handler))
        .route("/api/backup/status", get(backup_status_handler))
        .route("/api/backup/trigger", post(backup_trigger_handler))
        .route("/api/lnd/channels", get(lnd_channels_handler))
        .layer(middleware::from_fn(timeout_middleware));

    let sse_routes = Router::new().route("/api/notifications/sse", get(notifications_sse_handler));

    let fallback_route = Router::new().route(
        "/api/{*path}",
        get(|| async { error(StatusCode::NOT_FOUND, "Endpoint no disponible") }),
    );

    Router::new()
        .merge(api_routes)
        .merge(sse_routes)
        .merge(fallback_route)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit_middleware,
        ))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(state)
}

async fn timeout_middleware(request: axum::extract::Request, next: Next) -> Response {
    match tokio::time::timeout(Duration::from_secs(30), next.run(request)).await {
        Ok(response) => response,
        Err(_) => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(json!({"error": "Tiempo de espera agotado"})),
        )
            .into_response(),
    }
}

/// Reverse proxies whose `X-Forwarded-For` can be believed: `TRUSTED_PROXIES`,
/// a comma-separated list of addresses or CIDR ranges. Empty by default.
fn trusted_proxies() -> &'static [(IpAddr, u8)] {
    static TRUSTED: OnceLock<Vec<(IpAddr, u8)>> = OnceLock::new();
    TRUSTED.get_or_init(|| parse_proxies(&std::env::var("TRUSTED_PROXIES").unwrap_or_default()))
}

/// Addresses and CIDR ranges of a `TRUSTED_PROXIES` list; invalid entries are skipped.
pub fn parse_proxies(list: &str) -> Vec<(IpAddr, u8)> {
    list.split(',')
        .filter_map(|entry| {
            let entry = entry.trim();
            let (address, prefix) = match entry.split_once('/') {
                Some((address, prefix)) => (address, Some(prefix)),
                None => (entry, None),
            };
            let address: IpAddr = address.parse().ok()?;
            let max = if address.is_ipv4() { 32 } else { 128 };
            let prefix = match prefix {
                Some(prefix) => prefix.parse::<u8>().ok().filter(|p| *p <= max)?,
                None => max,
            };
            Some((address, prefix))
        })
        .collect()
}

fn in_range(ip: IpAddr, (network, prefix): (IpAddr, u8)) -> bool {
    match (ip, network) {
        (IpAddr::V4(ip), IpAddr::V4(network)) => {
            let mask = u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0);
            u32::from(ip) & mask == u32::from(network) & mask
        }
        (IpAddr::V6(ip), IpAddr::V6(network)) => {
            let mask = u128::MAX.checked_shl(128 - u32::from(prefix)).unwrap_or(0);
            u128::from(ip) & mask == u128::from(network) & mask
        }
        _ => false,
    }
}

/// Who a request counts against: the address that connected, unless it is a
/// trusted proxy; then the nearest `X-Forwarded-For` address that is not one.
/// A client cannot pick its own bucket by sending that header.
pub fn client_key(peer: Option<IpAddr>, headers: &HeaderMap, trusted: &[(IpAddr, u8)]) -> String {
    let Some(peer) = peer else {
        return "local".into();
    };
    let is_trusted = |ip: IpAddr| trusted.iter().any(|network| in_range(ip, *network));
    if !is_trusted(peer) {
        return peer.to_string();
    }
    let forwarded: Vec<IpAddr> = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|address| address.trim().parse().ok())
        .collect();
    forwarded
        .into_iter()
        .rev()
        .find(|ip| !is_trusted(*ip))
        .unwrap_or(peer)
        .to_string()
}

/// Who the request counts against (`client_key`), for the handlers that limit
/// on their own, like the login.
#[derive(Clone)]
struct ClientKey(String);

async fn rate_limit_middleware(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut request: axum::extract::Request,
    next: Next,
) -> Result<Response, Response> {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0.ip());
    let client_key = client_key(peer, &headers, trusted_proxies());

    if !state.rate_limiter.check(&client_key, 1.0) {
        let mut response = (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "Demasiadas solicitudes. Límite de tasa excedido."})),
        )
            .into_response();
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, "5".parse().unwrap());
        return Err(response);
    }

    request.extensions_mut().insert(ClientKey(client_key));
    Ok(next.run(request).await)
}

/// What answers without a session: enough to show the login page.
const PUBLIC_PATHS: [&str; 4] = [
    "/api/health",
    "/api/auth/session",
    "/api/auth/login",
    "/api/auth/logout",
];

async fn auth_middleware(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let path = request.uri().path();
    if !path.starts_with("/api/")
        || PUBLIC_PATHS.contains(&path)
        || state.auth.allows(request.headers())
    {
        return next.run(request).await;
    }
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({"error": "Inicia sesión para usar el panel", "login_required": true})),
    )
        .into_response()
}

async fn auth_session_handler(State(state): State<AppState>, headers: HeaderMap) -> Json<Value> {
    Json(json!({
        "mode": state.auth.mode().as_str(),
        "authenticated": state.auth.allows(&headers),
    }))
}

#[derive(Deserialize)]
struct LoginRequest {
    password: String,
}

async fn login_handler(
    State(state): State<AppState>,
    Extension(ClientKey(client)): Extension<ClientKey>,
    headers: HeaderMap,
    Json(payload): Json<LoginRequest>,
) -> Result<Response, Error> {
    verify_protection(&headers)?;
    let token = state
        .auth
        .login(&client, &payload.password)
        .map_err(|reason| match reason {
            LoginError::NotRequired => {
                error(StatusCode::BAD_REQUEST, "Este panel no usa contraseña")
            }
            LoginError::TooManyAttempts => error(
                StatusCode::TOO_MANY_REQUESTS,
                "Demasiados intentos. Espera un minuto y vuelve a probar.",
            ),
            LoginError::WrongPassword => error(StatusCode::UNAUTHORIZED, "Contraseña incorrecta"),
        })?;
    // verify_protection checked that the origin, when sent, is this panel.
    let secure = headers
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|origin| origin.starts_with("https://"));
    Ok((
        [(header::SET_COOKIE, auth::session_cookie(&token, secure))],
        Json(json!({"authenticated": true})),
    )
        .into_response())
}

async fn logout_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, Error> {
    verify_protection(&headers)?;
    state.auth.logout(&headers);
    Ok((
        [(header::SET_COOKIE, auth::expired_cookie())],
        Json(json!({"authenticated": state.auth.mode() != AuthMode::Password})),
    )
        .into_response())
}

pub fn verify_protection(headers: &HeaderMap) -> Result<(), Error> {
    if headers
        .get("x-requested-with")
        .and_then(|v| v.to_str().ok())
        != Some("mostro-community")
    {
        return Err(error(
            StatusCode::FORBIDDEN,
            "Falta protección de solicitud",
        ));
    }
    if let Some(origin) = headers.get("origin") {
        let host = headers
            .get("host")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let permitted = origin
            .to_str()
            .ok()
            .and_then(|s| url::Url::parse(s).ok())
            .is_some_and(|u| {
                ["http", "https"].contains(&u.scheme())
                    && u[url::Position::BeforeHost..url::Position::AfterPort] == *host
            });
        if !permitted {
            return Err(error(StatusCode::FORBIDDEN, "Origen no permitido"));
        }
    }
    Ok(())
}

async fn connection_info_handler(
    State(state): State<AppState>,
) -> Result<Json<connection::ConnectionInfo>, Error> {
    let store = state.store.lock().map_err(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Almacenamiento no disponible",
        )
    })?;
    let root = store.root().to_path_buf();
    Ok(Json(connection::get_connection_info(&root, &store)))
}

async fn community_card_handler(
    State(state): State<AppState>,
) -> Result<Json<connection::CommunityCard>, Error> {
    let store = state.store.lock().map_err(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Almacenamiento no disponible",
        )
    })?;
    let root = store.root().to_path_buf();
    connection::issue_community_card(&root, &store)
        .map(Json)
        .map_err(|reason| {
            error(
                StatusCode::CONFLICT,
                &reason.text_for(store.document.config.as_ref()),
            )
        })
}

/// The switch as saved, and what the publisher last did with it.
async fn card_publication_view(state: &AppState) -> Result<Json<PublicationView>, Error> {
    let enabled = state
        .store
        .lock()
        .map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Almacenamiento no disponible",
            )
        })?
        .document
        .publish_card;
    Ok(Json(state.card_publication.view(enabled).await))
}

async fn card_publication_handler(
    State(state): State<AppState>,
) -> Result<Json<PublicationView>, Error> {
    card_publication_view(&state).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CardPublicationRequest {
    enabled: bool,
}

/// Turns the publication of the community card on or off. The answer comes
/// back at once; the relays are dealt with by the worker, and its result is
/// read from the same address.
async fn card_publication_switch_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CardPublicationRequest>,
) -> Result<Json<PublicationView>, Error> {
    verify_protection(&headers)?;
    let saved = {
        let mut store = state.store.lock().map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Almacenamiento no disponible",
            )
        })?;
        if store.document.config.is_none() {
            return Err(error(
                StatusCode::CONFLICT,
                "Guarda primero las reglas de la comunidad: la tarjeta se genera a partir de ellas",
            ));
        }
        if store.document.publish_card == request.enabled {
            Ok(())
        } else {
            store.set_publish_card(request.enabled).map(|_| ())
        }
    };
    // The publisher follows what the store holds now, whatever the write
    // reported: a failure after the file was replaced leaves the new value.
    state.card_publication.request();
    saved.map_err(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "No se pudo guardar la configuración",
        )
    })?;
    card_publication_view(&state).await
}

async fn community_presets_handler() -> Json<Vec<crate::config::RegionalPreset>> {
    Json(crate::config::get_regional_presets())
}

#[derive(Deserialize, Default)]
struct GenerateIdentityRequest {
    overwrite: Option<bool>,
}

async fn identity_generate_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Value>, Error> {
    verify_protection(&headers)?;
    let root = state
        .store
        .lock()
        .map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Almacenamiento no disponible",
            )
        })?
        .root()
        .to_path_buf();

    let overwrite = if !body.is_empty() {
        let req: GenerateIdentityRequest = serde_json::from_slice(&body).unwrap_or_default();
        req.overwrite.unwrap_or(false)
    } else {
        false
    };

    let generated = identity::generate_with_overwrite(&root, overwrite);
    // Also when it failed: an overwrite may have removed the key the card was signed with.
    state.card_publication.request();
    let (nsec, npub) = generated.map_err(|e| error(StatusCode::BAD_REQUEST, e))?;

    let config_opt = state
        .store
        .lock()
        .ok()
        .and_then(|s| s.document.config.clone());
    if let Some(config) = config_opt {
        let _ = state.monitor_tx.send(orders::MonitorCommand {
            config,
            npub: Some(npub.clone()),
        });
    }

    state
        .notifications
        .publish(Notification::system_alert(
            "Identidad Mostro generada",
            &format!("Nueva clave pública inicializada: {npub}"),
            "info",
            None,
        ))
        .await;

    Ok(Json(json!({
        "status": "ok",
        "nsec": nsec,
        "npub": npub
    })))
}

#[derive(Deserialize)]
struct ImportIdentityRequest {
    nsec: String,
    overwrite: Option<bool>,
}

async fn identity_import_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ImportIdentityRequest>,
) -> Result<Json<Value>, Error> {
    verify_protection(&headers)?;
    let root = state
        .store
        .lock()
        .map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Almacenamiento no disponible",
            )
        })?
        .root()
        .to_path_buf();

    let overwrite = payload.overwrite.unwrap_or(false);
    let imported = identity::import_nsec_with_overwrite(&root, &payload.nsec, overwrite);
    // Also when it failed: an overwrite may have removed the key the card was signed with.
    state.card_publication.request();
    let npub = imported.map_err(|e| error(StatusCode::BAD_REQUEST, e))?;

    let config_opt = state
        .store
        .lock()
        .ok()
        .and_then(|s| s.document.config.clone());
    if let Some(config) = config_opt {
        let _ = state.monitor_tx.send(orders::MonitorCommand {
            config,
            npub: Some(npub.clone()),
        });
    }

    state
        .notifications
        .publish(Notification::system_alert(
            "Identidad Mostro importada",
            &format!("Clave pública vinculada: {npub}"),
            "info",
            None,
        ))
        .await;

    Ok(Json(json!({
        "status": "ok",
        "npub": npub
    })))
}

#[derive(Deserialize)]
struct ActivateDaemonRequest {
    lnd_grpc_origin: Option<String>,
}

async fn daemon_status_handler(
    State(state): State<AppState>,
) -> Result<Json<daemon::DaemonReport>, Error> {
    let root = state
        .store
        .lock()
        .map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Almacenamiento no disponible",
            )
        })?
        .root()
        .to_path_buf();
    let node_info = state.orders.read().await.node_info.clone();
    Ok(Json(
        daemon::report_with_node_info(&root, &state.integrations, node_info).await,
    ))
}

async fn daemon_activate_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<daemon::ActivationResult>, Error> {
    verify_protection(&headers)?;
    let root = state
        .store
        .lock()
        .map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Almacenamiento no disponible",
            )
        })?
        .root()
        .to_path_buf();

    let payload: Option<ActivateDaemonRequest> = if body.is_empty() {
        None
    } else {
        Some(serde_json::from_slice(&body).map_err(|e| {
            error(
                StatusCode::BAD_REQUEST,
                &format!("Cuerpo de solicitud inválido: {e}"),
            )
        })?)
    };

    let lnd_origin = payload
        .and_then(|p| p.lnd_grpc_origin)
        .or_else(daemon::lnd_grpc_origin_from_env)
        .ok_or_else(|| {
            error(
                StatusCode::BAD_REQUEST,
                "Falta la dirección gRPC de LND: define MOSTRO_LND_GRPC_HOST (por ejemplo https://lnd:10009) en el contenedor del panel",
            )
        })?;

    daemon::activate(&root, &lnd_origin)
        .map(Json)
        .map_err(|e| error(StatusCode::BAD_REQUEST, e))
}

async fn daemon_deactivate_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, Error> {
    verify_protection(&headers)?;
    let root = state
        .store
        .lock()
        .map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Almacenamiento no disponible",
            )
        })?
        .root()
        .to_path_buf();
    daemon::deactivate(&root)
        .map(|()| Json(json!({"status": "deactivated"})))
        .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationRunRequest {
    pub scenario: Option<simulation::SimulationScenario>,
    pub trade_sats: Option<u64>,
}

async fn simulation_scenarios_handler() -> Json<Value> {
    Json(json!([
        {
            "id": "happy_path",
            "label": "Operación completada",
            "name": "Operación completada",
            "description": "Una venta de principio a fin: publicación, garantías, depósito del vendedor, pago fiat y liberación de los sats.",
            "is_default": true
        },
        {
            "id": "dispute_settled_for_buyer",
            "label": "Disputa resuelta a favor del comprador",
            "name": "Disputa resuelta a favor del comprador",
            "description": "El mediador comprueba que el pago fiat existió: Mostro cobra el depósito del vendedor y paga al comprador.",
            "is_default": false
        },
        {
            "id": "dispute_refunded_to_seller",
            "label": "Disputa resuelta con devolución al vendedor",
            "name": "Disputa resuelta con devolución al vendedor",
            "description": "El mediador comprueba que el pago fiat no existió: Mostro cancela el depósito y los sats vuelven al vendedor.",
            "is_default": false
        },
        {
            "id": "seller_cancellation",
            "label": "Cancelación por el vendedor antes de la toma",
            "name": "Cancelación por el vendedor antes de la toma",
            "description": "El vendedor retira su orden antes de que alguien la tome.",
            "is_default": false
        }
    ]))
}

async fn simulation_run_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<simulation::SimulationReport>, Error> {
    verify_protection(&headers)?;

    let payload: Option<SimulationRunRequest> = if body.is_empty() {
        None
    } else {
        Some(serde_json::from_slice(&body).map_err(|e| {
            error(
                StatusCode::BAD_REQUEST,
                &format!("Cuerpo de solicitud inválido: {e}"),
            )
        })?)
    };

    let (scenario, trade_sats) = match payload {
        Some(p) => (
            p.scenario
                .unwrap_or(simulation::SimulationScenario::HappyPath),
            p.trade_sats,
        ),
        None => (simulation::SimulationScenario::HappyPath, None),
    };

    if let Some(0) = trade_sats {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "Monto de intercambio debe ser mayor a cero",
        ));
    }

    let store = state.store.lock().map_err(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Almacenamiento no disponible",
        )
    })?;

    let config = store.document.config.as_ref().ok_or_else(|| {
        error(
            StatusCode::BAD_REQUEST,
            "Falta configurar la comunidad antes de simular",
        )
    })?;

    simulation::run_simulation(config, None, scenario, trade_sats)
        .map(Json)
        .map_err(|e| error(StatusCode::BAD_REQUEST, e))
}

/// Liveness of the API itself. `version` is the released package version when
/// the image was built by the release workflow, otherwise the crate version.
async fn health(State(state): State<AppState>) -> Json<Value> {
    let release = option_env!("MANAGER_VERSION")
        .map(|v| v.trim_start_matches('v'))
        .filter(|v| !v.is_empty() && *v != "dev");
    Json(json!({
        "status": "ok",
        "version": release.unwrap_or(env!("CARGO_PKG_VERSION")),
        "mode": if release.is_some() { "release" } else { "development" },
        "mostro_pinned_version": daemon::MOSTRO_VERSION,
        "auth": state.auth.mode().as_str(),
        "platform": platform(),
    }))
}

/// Where the panel runs, for the commands and paths it shows: `umbrel`, or
/// `docker` for everything else. Set by the package with `MCM_PLATFORM`.
fn platform() -> &'static str {
    match std::env::var("MCM_PLATFORM").as_deref().map(str::trim) {
        Ok("umbrel") => "umbrel",
        _ => "docker",
    }
}

async fn dashboard(State(state): State<AppState>) -> Json<Value> {
    let (mut mostro, lightning) =
        tokio::join!(state.integrations.mostro(), state.integrations.lightning());
    let root = state.store.lock().ok().map(|s| s.root().to_path_buf());
    let (is_active, active_rev) = match &root {
        Some(root) => {
            let active_dir = root.join("active");
            let has_settings = active_dir.join("settings.toml").is_file();
            let rev = std::fs::read(active_dir.join("status.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<daemon::ActiveStatus>(&b).ok())
                .map(|s| s.revision);
            (has_settings, rev)
        }
        None => (false, None),
    };
    let running_for = root.as_deref().and_then(daemon::running_for_secs);
    // The daemon's own info event on the relays is the evidence that a node
    // with this identity is serving clients. Without a recent one the panel
    // reports what it knows and no more.
    let (announced, announced_age) = {
        let orders = state.orders.read().await;
        let now = nostr::Timestamp::now().as_secs();
        (orders.node_info.clone(), orders.node_info_age_secs(now))
    };
    // The same judgement as `/api/daemon/status`: what the previous process
    // announced does not open the market of the one that replaced it.
    let announcement = daemon::AnnouncementAge::judge(announced_age, running_for);
    let announced_fresh = announcement.fresh();
    let in_maintenance = announced
        .as_ref()
        .is_some_and(|info| info.tags.get("maintenance_mode").map(String::as_str) == Some("true"));
    if state.integrations.mostro_rpc.is_none() {
        if let (true, Some(info)) = (announced_fresh, announced.as_ref()) {
            mostro = json!({
                "status": if in_maintenance { "warning" } else { "online" },
                "detail": if in_maintenance {
                    "El daemon anuncia modo mantenimiento: no acepta órdenes nuevas".to_string()
                } else {
                    format!(
                        "El daemon anunció su información en los relays hace {} s",
                        announced_age.unwrap_or_default()
                    )
                },
                "version": info.mostro_version,
                "announced_age_secs": announced_age,
                "configured_revision": active_rev
            });
        } else if is_active {
            mostro = json!({
                "status": "unknown",
                "detail": if announcement.before_start {
                    "El daemon acaba de arrancar y aún no se ha anunciado: el anuncio que conservan los relays es anterior a este arranque"
                } else {
                    "Configuración guardada; el daemon no ha anunciado su información en los relays recientemente"
                },
                "configured_revision": active_rev
            });
        }
    }
    Json(json!({"mostro":mostro,"lightning":lightning,
        "bitcoin":{"status":"unknown","detail":"Verificación directa de Bitcoin pendiente; el estado de LND no la sustituye"},
        "configuration_active":is_active,
        "market_started":announced_fresh && !in_maintenance}))
}

#[derive(Deserialize, Default)]
pub struct LndChannelsQuery {
    pub mock: Option<bool>,
}

async fn lnd_channels_handler(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<LndChannelsQuery>,
) -> Json<Value> {
    let force_mock = query.mock.unwrap_or(false);
    Json(state.integrations.channels(force_mock).await)
}

async fn community(State(state): State<AppState>) -> Result<Json<Document>, Error> {
    let store = state.store.lock().map_err(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Almacenamiento no disponible",
        )
    })?;
    Ok(Json(store.document.clone()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveRequest {
    revision: u64,
    config: Configuration,
}

async fn save_community(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SaveRequest>,
) -> Result<Json<Document>, Error> {
    verify_protection(&headers)?;
    request
        .config
        .validate_for_save()
        .map_err(|e| error(StatusCode::UNPROCESSABLE_ENTITY, e))?;
    let (result, root) = {
        let mut store = state.store.lock().map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Almacenamiento no disponible",
            )
        })?;
        if request.revision != store.document.revision {
            return Err(error(
                StatusCode::CONFLICT,
                "La configuración cambió; recarga antes de guardar",
            ));
        }
        let root = store.root.clone();
        let save_res = store.save(request.config.clone()).map(Json).map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "No se pudo guardar la configuración",
            )
        });
        (save_res, root)
    };

    // The card may have changed with the rules: the publisher decides. It is
    // asked even after a failed save, which can leave the new rules in place.
    state.card_publication.request();

    if result.is_ok() {
        let npub = identity::read_public_key(&root).ok().flatten();
        let _ = state.monitor_tx.send(orders::MonitorCommand {
            config: request.config,
            npub,
        });

        // Restarts the daemon only if the rendered settings changed.
        if let Err(reason) = daemon::sync_with_recorded_origin(&root) {
            state
                .notifications
                .publish(Notification::system_alert(
                    "Activación incompleta",
                    &format!(
                        "Las reglas se guardaron, pero no llegaron a aplicarse al nodo: {reason}. Comprueba en la página Nodo Mostro qué revisión está activa y pulsa «Aplicar y reiniciar»."
                    ),
                    "critical",
                    None,
                ))
                .await;
        }
    }

    result
}

async fn notifications_list_handler(State(state): State<AppState>) -> Json<Vec<Notification>> {
    let recent = state.notifications.get_recent(50).await;
    Json(recent)
}

async fn notifications_sse_handler(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.notifications.subscribe();

    let stream = stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(notif) => {
                    let json = serde_json::to_string(&notif).unwrap_or_default();
                    let event = Event::default().event("notification").data(json);
                    return Some((Ok(event), rx));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    return None;
                }
            }
        }
    });

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

async fn backup_status_handler(State(state): State<AppState>) -> Json<AutoBackupState> {
    let st = state.backup_state.read().await;
    Json(st.clone())
}

#[derive(Deserialize)]
pub struct BackupTriggerRequest {
    pub passphrase: Option<String>,
}

async fn backup_trigger_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Option<Json<BackupTriggerRequest>>,
) -> Result<Json<Value>, Error> {
    verify_protection(&headers)?;

    let root = state
        .store
        .lock()
        .map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Almacenamiento no disponible",
            )
        })?
        .root()
        .to_path_buf();

    let passphrase_str = payload
        .and_then(|p| p.0.passphrase)
        .or_else(|| std::env::var("BACKUP_PASSPHRASE").ok())
        .ok_or_else(|| {
            error(
                StatusCode::BAD_REQUEST,
                "Se requiere frase de cifrado (en cuerpo o variable BACKUP_PASSPHRASE)",
            )
        })?;

    let target_dir = PathBuf::from(
        std::env::var("BACKUP_OFFSITE_DIR").unwrap_or_else(|_| "/data/backup".into()),
    );
    let target_dir = if !target_dir.exists() {
        if backup::ensure_private_dir(&target_dir).is_err() {
            root.join("backups")
        } else {
            target_dir
        }
    } else {
        target_dir
    };

    // A manual backup prunes like the automatic ones, with the same limit.
    let retention_count = state.backup_state.read().await.retention_count;
    let summary = backup::run_auto_backup_cycle(
        &root,
        &target_dir,
        retention_count,
        age::secrecy::SecretString::from(passphrase_str),
    )
    .map_err(|e| error(StatusCode::BAD_REQUEST, e))?;

    let backups = backup::list_backups(&target_dir);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    {
        let mut st = state.backup_state.write().await;
        st.last_run_timestamp = Some(now);
        st.last_run_success = Some(true);
        st.last_error = None;
        st.last_backup_path = Some(summary.path.display().to_string());
        st.backups = backups;
    }

    state
        .notifications
        .publish(Notification::backup_alert(
            "Respaldo manual completado",
            &format!("Respaldo guardado en {}", summary.path.display()),
            true,
            Some(json!({
                "path": summary.path.display().to_string(),
                "revision": summary.revision,
                "npub": summary.npub
            })),
        ))
        .await;

    Ok(Json(json!({
        "status": "ok",
        "path": summary.path.display().to_string(),
        "database_path": summary.database_path.as_ref().map(|p| p.display().to_string()),
        "revision": summary.revision,
        "npub": summary.npub
    })))
}
