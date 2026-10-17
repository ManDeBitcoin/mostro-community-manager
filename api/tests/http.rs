use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use mostro_community_api::{AppState, adapters::Integrations, router, store::Store};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
fn setup() -> (tempfile::TempDir, axum::Router) {
    let dir = tempfile::tempdir().unwrap();
    let app = app_for(dir.path());
    (dir, app)
}
/// The API over whatever `root` already holds.
fn app_for(root: &std::path::Path) -> axum::Router {
    app_with_orders(root).0
}
/// The same, with the cache its relay monitor would fill.
fn app_with_orders(
    root: &std::path::Path,
) -> (axum::Router, mostro_community_api::orders::SharedOrders) {
    let initial_config: mostro_community_api::config::Configuration =
        serde_json::from_str(include_str!("fixtures/community.json")).unwrap();
    let (tx, _rx) = tokio::sync::watch::channel(mostro_community_api::orders::MonitorCommand {
        config: initial_config,
        npub: None,
    });
    let orders = Arc::new(tokio::sync::RwLock::new(
        mostro_community_api::orders::OrdersCache::new(),
    ));
    let app = router(AppState::new(
        Arc::new(Mutex::new(Store::open(root.into()).unwrap())),
        Integrations::default(),
        orders.clone(),
        Arc::new(tokio::sync::RwLock::new(
            mostro_community_api::chat::ChatCache::new(),
        )),
        tx,
    ));
    (app, orders)
}
fn save(revision: u64, header: bool, origin: &str, config: serde_json::Value) -> Request<Body> {
    let mut builder = Request::builder()
        .method("PUT")
        .uri("/api/community")
        .header("content-type", "application/json")
        .header("host", "localhost:5173")
        .header("origin", origin);
    if header {
        builder = builder.header("x-requested-with", "mostro-community");
    }
    builder
        .body(Body::from(
            serde_json::json!({"revision":revision,"config":config}).to_string(),
        ))
        .unwrap()
}
fn config() -> serde_json::Value {
    serde_json::from_str(include_str!("fixtures/community.json")).unwrap()
}
#[tokio::test]
async fn mutation_requires_same_origin_custom_header_and_fresh_revision() {
    let (_dir, app) = setup();
    assert_eq!(
        app.clone()
            .oneshot(save(0, false, "http://localhost:5173", config()))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.clone()
            .oneshot(save(0, true, "https://evil.example", config()))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.clone()
            .oneshot(save(0, true, "http://localhost:5173", config()))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        app.clone()
            .oneshot(save(0, true, "http://localhost:5173", config()))
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.oneshot(save(1, true, "http://localhost:5173", config()))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
}
#[tokio::test]
async fn invalid_configuration_is_not_persisted() {
    let (dir, app) = setup();
    let mut invalid = config();
    invalid["market"]["min_trade_sats"] = 0.into();
    assert_eq!(
        app.oneshot(save(0, true, "http://localhost:5173", invalid))
            .await
            .unwrap()
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert!(!dir.path().join("community.json").exists());
}
#[tokio::test]
async fn secrets_and_unknown_fields_are_rejected() {
    let (dir, app) = setup();
    let mut invalid = config();
    invalid["nostr"]["nsec"] = "secret".into();
    assert_eq!(
        app.oneshot(save(0, true, "http://localhost:5173", invalid))
            .await
            .unwrap()
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert!(!dir.path().join("community.json").exists());
}
#[tokio::test]
async fn health_never_fabricates_connected_services() {
    let (_dir, app) = setup();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/dashboard")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(value["mostro"]["status"], "unconfigured");
    assert_eq!(value["lightning"]["status"], "unconfigured");
    assert_eq!(value["bitcoin"]["status"], "unknown");
    assert_eq!(value["market_started"], false);
}

#[tokio::test]
async fn saved_settings_do_not_fabricate_daemon_health_or_version() {
    let (dir, app) = setup();
    let active = dir.path().join("active");
    std::fs::create_dir(&active).unwrap();
    std::fs::write(active.join("settings.toml"), "[mostro]\n").unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/dashboard")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(value["configuration_active"], true);
    assert_eq!(value["mostro"]["status"], "unknown");
    assert!(value["mostro"].get("version").is_none());
    assert!(value["mostro"]["configured_revision"].is_null());
    assert_eq!(value["market_started"], false);
}

#[tokio::test]
async fn imported_identity_is_not_exposed_over_http() {
    use nostr::{Keys, SecretKey, ToBech32};
    let (dir, app) = setup();
    let keys = Keys::new(SecretKey::from_slice(&[3; 32]).unwrap());
    let secret = keys.secret_key().to_bech32().unwrap();
    let public = keys.public_key().to_bech32().unwrap();
    mostro_community_api::identity::import(dir.path(), &secret, &public).unwrap();
    for path in [
        "/api/community",
        "/api/dashboard",
        "/api/connection",
        "/api/identity",
        "/api/identity/mostro.nsec",
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        if path.starts_with("/api/identity") {
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert!(!String::from_utf8_lossy(&body).contains(&secret));
    }
}

#[tokio::test]
async fn daemon_endpoints_enforce_protection_and_report_state() {
    let (_dir, app) = setup();

    // GET daemon status requires no CSRF header and succeeds
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/daemon/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["state"], "unconfigured");

    // PUT daemon activate fails without custom header
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/daemon/activate")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // POST daemon start fails without custom header
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/daemon/start")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // PUT daemon deactivate fails without custom header
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/daemon/deactivate")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    // POST daemon stop fails without custom header
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/daemon/stop")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn daemon_activation_flow_and_dashboard_status() {
    use nostr::{Keys, SecretKey, ToBech32};
    use std::os::unix::fs::PermissionsExt;
    let (dir, app) = setup();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();

    // 1. Initial dashboard reports market_started = false and mostro unconfigured
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/dashboard")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["market_started"], false);
    assert_eq!(body["mostro"]["status"], "unconfigured");

    // 2. Save community draft
    let res = app
        .clone()
        .oneshot(save(0, true, "http://localhost:5173", config()))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 3. Import private identity
    let keys = Keys::new(SecretKey::from_slice(&[42; 32]).unwrap());
    let secret = keys.secret_key().to_bech32().unwrap();
    let public = keys.public_key().to_bech32().unwrap();
    mostro_community_api::identity::import(dir.path(), &secret, &public).unwrap();

    // 4. Verify daemon status can_activate = true, state = configured_standby
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/daemon/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["state"], "configured_standby");
    assert_eq!(body["can_activate"], true);

    // 5. POST /api/daemon/start activates daemon
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/daemon/start")
                .header("x-requested-with", "mostro-community")
                .header("host", "localhost:5173")
                .header("origin", "http://localhost:5173")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"lnd_grpc_origin":"https://127.0.0.1:10009"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["revision"], 1);
    assert_eq!(body["settings_sha256"].as_str().unwrap().len(), 64);

    // 6. Verify settings.toml exists and active status
    assert!(dir.path().join("active").join("settings.toml").is_file());

    // 7. Saving settings cannot establish daemon health or a running market.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/dashboard")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["configuration_active"], true);
    assert_eq!(body["market_started"], false);
    assert_eq!(body["mostro"]["status"], "unknown");
    assert_eq!(body["mostro"]["configured_revision"], 1);
    assert!(body["mostro"].get("version").is_none());

    // The explicit activation asked the supervisor to start the daemon.
    let wake = dir.path().join(".standby_wake");
    let status_path = dir.path().join("active").join("status.json");
    let active_revision = || -> serde_json::Value {
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(&status_path).unwrap()).unwrap()
            ["revision"]
            .clone()
    };
    assert!(wake.exists());
    std::fs::remove_file(&wake).unwrap();

    // A draft that renders the same settings.toml (a payment-method label) is
    // recorded as active without restarting a daemon that may have trades in
    // flight.
    let mut relabelled = config();
    relabelled["payment_methods"][0]["label"] = serde_json::json!("Transferencia SEPA");
    let res = app
        .clone()
        .oneshot(save(1, true, "http://localhost:5173", relabelled))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(active_revision(), 2);
    assert!(
        !wake.exists(),
        "an unchanged settings.toml restarted the daemon"
    );

    // A draft that changes settings.toml is applied and restarts it.
    let mut updated_config = config();
    updated_config["market"]["fee_bps"] = serde_json::json!(75);
    let res = app
        .clone()
        .oneshot(save(
            2,
            true,
            "http://localhost:5173",
            updated_config.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let active_settings =
        std::fs::read_to_string(dir.path().join("active").join("settings.toml")).unwrap();
    assert!(active_settings.contains("fee = 0.0075"));
    assert_eq!(active_revision(), 3);
    assert!(wake.exists());

    // Without the activation record the LND origin of the running daemon is
    // unknown: the draft is saved but not applied, and the operator is told.
    std::fs::remove_file(&wake).unwrap();
    std::fs::remove_file(&status_path).unwrap();
    updated_config["market"]["fee_bps"] = serde_json::json!(90);
    let res = app
        .clone()
        .oneshot(save(3, true, "http://localhost:5173", updated_config))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(!wake.exists());
    let unchanged =
        std::fs::read_to_string(dir.path().join("active").join("settings.toml")).unwrap();
    assert!(unchanged.contains("fee = 0.0075"));
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/notifications")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let notifications: serde_json::Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(
        notifications
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["level"] == "critical" && n["title"] == "Activación incompleta")
    );

    // 8. POST /api/daemon/stop deactivates daemon
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/daemon/stop")
                .header("x-requested-with", "mostro-community")
                .header("host", "localhost:5173")
                .header("origin", "http://localhost:5173")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(!dir.path().join("active").join("settings.toml").exists());

    // 9. Dashboard no longer reports a prepared configuration.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/dashboard")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["market_started"], false);
    assert_eq!(body["configuration_active"], false);
    assert_eq!(body["mostro"]["status"], "unconfigured");
}

fn sim_request(
    header: bool,
    origin: Option<&str>,
    body: Option<serde_json::Value>,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/api/simulation/run")
        .header("content-type", "application/json")
        .header("host", "localhost:5173");
    if header {
        builder = builder.header("x-requested-with", "mostro-community");
    }
    if let Some(o) = origin {
        builder = builder.header("origin", o);
    }
    let body_bytes = match body {
        Some(v) => Body::from(v.to_string()),
        None => Body::empty(),
    };
    builder.body(body_bytes).unwrap()
}

#[tokio::test]
async fn simulation_contract_enforces_protection_headers() {
    let (_dir, app) = setup();

    // 1. Missing x-requested-with -> 403
    let res = app
        .clone()
        .oneshot(sim_request(false, Some("http://localhost:5173"), None))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // 2. Disallowed origin -> 403
    let res = app
        .clone()
        .oneshot(sim_request(true, Some("https://attacker.example"), None))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn simulation_contract_requires_configured_community() {
    let (_dir, app) = setup();

    // Unconfigured store -> 400 Bad Request
    let res = app
        .oneshot(sim_request(true, Some("http://localhost:5173"), None))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let body: serde_json::Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(
        body["error"],
        "Falta configurar la comunidad antes de simular"
    );
}

#[tokio::test]
async fn simulation_contract_accepts_defaults_with_valid_config() {
    let (_dir, app) = setup();

    // Configure community first
    let save_res = app
        .clone()
        .oneshot(save(0, true, "http://localhost:5173", config()))
        .await
        .unwrap();
    assert_eq!(save_res.status(), StatusCode::OK);

    // Empty body -> 200 OK, defaults to happy_path
    let res = app
        .clone()
        .oneshot(sim_request(true, Some("http://localhost:5173"), None))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["scenario"], "happy_path");
    assert_eq!(body["simulation_mode"], "synthetic_dry_run");
    assert!(body["disclaimer"].as_str().unwrap().contains("sintética"));
    assert_eq!(body["is_success"], true);

    // Empty JSON object -> 200 OK, defaults to happy_path
    let res = app
        .oneshot(sim_request(
            true,
            Some("http://localhost:5173"),
            Some(serde_json::json!({})),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["scenario"], "happy_path");
}

#[tokio::test]
async fn simulation_contract_rejects_unknown_scenarios_and_fields() {
    let (_dir, app) = setup();
    app.clone()
        .oneshot(save(0, true, "http://localhost:5173", config()))
        .await
        .unwrap();

    // Unknown scenario enum variant -> 400 Bad Request (NOT silently defaulted to happy_path!)
    let res = app
        .clone()
        .oneshot(sim_request(
            true,
            Some("http://localhost:5173"),
            Some(serde_json::json!({"scenario": "non_existent_scenario"})),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // Numeric scenario type mismatch -> 400 Bad Request
    let res = app
        .clone()
        .oneshot(sim_request(
            true,
            Some("http://localhost:5173"),
            Some(serde_json::json!({"scenario": 999})),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // Unknown extra field -> 400 Bad Request (deny_unknown_fields)
    let res = app
        .oneshot(sim_request(
            true,
            Some("http://localhost:5173"),
            Some(serde_json::json!({
                "scenario": "happy_path",
                "malicious_extra": "rejected"
            })),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn simulation_contract_rejects_invalid_trade_amounts() {
    let (_dir, app) = setup();
    app.clone()
        .oneshot(save(0, true, "http://localhost:5173", config()))
        .await
        .unwrap();

    // Zero trade sats -> 400 Bad Request
    let res = app
        .clone()
        .oneshot(sim_request(
            true,
            Some("http://localhost:5173"),
            Some(serde_json::json!({"trade_sats": 0})),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // Negative trade sats -> 400 Bad Request
    let res = app
        .clone()
        .oneshot(sim_request(
            true,
            Some("http://localhost:5173"),
            Some(serde_json::json!({"trade_sats": -100})),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // Decimal trade sats -> 400 Bad Request
    let res = app
        .clone()
        .oneshot(sim_request(
            true,
            Some("http://localhost:5173"),
            Some(serde_json::json!({"trade_sats": 50000.5})),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // String trade sats -> 400 Bad Request
    let res = app
        .clone()
        .oneshot(sim_request(
            true,
            Some("http://localhost:5173"),
            Some(serde_json::json!({"trade_sats": "fifty_thousand"})),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // Below minimum (min_trade_sats is 100 in fixtures/community.json) -> 400
    let res = app
        .clone()
        .oneshot(sim_request(
            true,
            Some("http://localhost:5173"),
            Some(serde_json::json!({"trade_sats": 50})),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // Above maximum (max_trade_sats is 50000 in fixtures/community.json) -> 400
    let res = app
        .oneshot(sim_request(
            true,
            Some("http://localhost:5173"),
            Some(serde_json::json!({"trade_sats": 100_000})),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn simulation_scenarios_contract_returns_metadata() {
    let (_dir, app) = setup();
    let res = app
        .oneshot(
            Request::builder()
                .uri("/api/simulation/scenarios")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(body.is_array());
    assert_eq!(body.as_array().unwrap().len(), 4);
    assert_eq!(body[0]["id"], "happy_path");
    assert_eq!(body[0]["is_default"], true);
}

#[tokio::test]
async fn orders_endpoint_reports_snapshot_without_secrets() {
    let (_dir, app) = setup();
    let res = app
        .oneshot(
            Request::builder()
                .uri("/api/orders")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8_lossy(&bytes);
    assert!(!body_str.contains("nsec"));
    assert!(!body_str.contains("private"));
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(body.is_object());
    assert!(body["state"].is_string());
    assert!(body["is_stale"].is_boolean());
    assert!(body["orders"].is_array());
    assert!(body["relays"].is_array());
}

#[tokio::test]
async fn notifications_endpoint_returns_recent_and_sse_headers() {
    let (_dir, app) = setup();

    // 1. GET /api/notifications -> 200 OK JSON array
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/notifications")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(body.is_array());

    // 2. GET /api/notifications/sse -> 200 OK text/event-stream
    let res_sse = app
        .oneshot(
            Request::builder()
                .uri("/api/notifications/sse")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res_sse.status(), StatusCode::OK);
    let content_type = res_sse
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(content_type.contains("text/event-stream"));
}

#[tokio::test]
async fn rate_limiting_enforces_limits_and_returns_429() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, _rx) = tokio::sync::watch::channel(mostro_community_api::orders::MonitorCommand {
        config: serde_json::from_str(include_str!("fixtures/community.json")).unwrap(),
        npub: None,
    });
    let mut state = AppState::new(
        Arc::new(Mutex::new(Store::open(dir.path().into()).unwrap())),
        Integrations::default(),
        Arc::new(tokio::sync::RwLock::new(
            mostro_community_api::orders::OrdersCache::new(),
        )),
        Arc::new(tokio::sync::RwLock::new(
            mostro_community_api::chat::ChatCache::new(),
        )),
        tx,
    );
    // Strict limiter: 2 tokens max, 0 refill rate
    state.rate_limiter = Arc::new(mostro_community_api::RateLimiter::new(2.0, 0.0));
    let app = router(state);

    // Request 1: OK
    let res1 = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res1.status(), StatusCode::OK);

    // Request 2: OK
    let res2 = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res2.status(), StatusCode::OK);

    // Request 3: Exceeded -> 429 Too Many Requests
    let res3 = app
        .oneshot(
            Request::builder()
                .uri("/api/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res3.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        res3.headers()
            .get("retry-after")
            .and_then(|h| h.to_str().ok()),
        Some("5")
    );
}

#[tokio::test]
async fn backup_status_and_trigger_contracts() {
    let (_dir, app) = setup();

    // 1. GET /api/backup/status -> 200 OK
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/backup/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(body.is_object());
    assert!(body["interval_secs"].is_number());
    assert!(body["backups"].is_array());

    // 2. POST /api/backup/trigger without protection -> 403 Forbidden
    let res_no_csrf = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/backup/trigger")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"passphrase":"some-passphrase"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res_no_csrf.status(), StatusCode::FORBIDDEN);

    // 3. POST /api/backup/trigger with CSRF but missing passphrase -> 400 Bad Request
    let res_missing_pass = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/backup/trigger")
                .header("content-type", "application/json")
                .header("x-requested-with", "mostro-community")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res_missing_pass.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn lnd_channels_endpoint_contract() {
    let (_dir, app) = setup();

    // 1. GET /api/lnd/channels?mock=true -> 200 OK with mock channels
    let res_mock = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/lnd/channels?mock=true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res_mock.status(), StatusCode::OK);
    let body_mock: serde_json::Value =
        serde_json::from_slice(&res_mock.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body_mock["status"], "online");
    assert_eq!(body_mock["is_mock"], true);
    assert_eq!(body_mock["num_active_channels"], 2);
    assert_eq!(body_mock["inbound_sufficient"], true);
    assert_eq!(body_mock["channels"].as_array().unwrap().len(), 2);

    // 2. GET /api/lnd/channels without mock -> 200 OK (unconfigured when no credentials)
    let res_default = app
        .oneshot(
            Request::builder()
                .uri("/api/lnd/channels")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res_default.status(), StatusCode::OK);
    let body_default: serde_json::Value =
        serde_json::from_slice(&res_default.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert_eq!(body_default["status"], "unconfigured");
    assert_eq!(body_default["channels"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_community_presets_and_identity_endpoints() {
    let (_dir, app) = setup();

    let res_presets = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/community/presets")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res_presets.status(), StatusCode::OK);
    let presets: serde_json::Value =
        serde_json::from_slice(&res_presets.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert_eq!(presets.as_array().unwrap().len(), 3);
    assert_eq!(presets[0]["id"], "latam");
    assert_eq!(presets[1]["id"], "europe");
    assert_eq!(presets[2]["id"], "global");

    let req_unprotected = Request::builder()
        .method("POST")
        .uri("/api/identity/generate")
        .header("host", "localhost:5173")
        .body(Body::empty())
        .unwrap();
    let res_unprotected = app.clone().oneshot(req_unprotected).await.unwrap();
    assert_eq!(res_unprotected.status(), StatusCode::FORBIDDEN);

    let req_gen = Request::builder()
        .method("POST")
        .uri("/api/identity/generate")
        .header("x-requested-with", "mostro-community")
        .header("host", "localhost:5173")
        .body(Body::empty())
        .unwrap();
    let res_gen = app.clone().oneshot(req_gen).await.unwrap();
    assert_eq!(res_gen.status(), StatusCode::OK);
    let gen_body: serde_json::Value =
        serde_json::from_slice(&res_gen.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let nsec = gen_body["nsec"].as_str().unwrap().to_string();
    let npub = gen_body["npub"].as_str().unwrap().to_string();
    assert!(nsec.starts_with("nsec1"));
    assert!(npub.starts_with("npub1"));

    let req_gen_duplicate = Request::builder()
        .method("POST")
        .uri("/api/identity/generate")
        .header("x-requested-with", "mostro-community")
        .header("host", "localhost:5173")
        .body(Body::empty())
        .unwrap();
    let res_dup = app.clone().oneshot(req_gen_duplicate).await.unwrap();
    assert_eq!(res_dup.status(), StatusCode::BAD_REQUEST);

    let (_dir2, app2) = setup();
    let req_import = Request::builder()
        .method("POST")
        .uri("/api/identity/import")
        .header("content-type", "application/json")
        .header("x-requested-with", "mostro-community")
        .header("host", "localhost:5173")
        .body(Body::from(serde_json::json!({"nsec": nsec}).to_string()))
        .unwrap();
    let res_import = app2.clone().oneshot(req_import).await.unwrap();
    assert_eq!(res_import.status(), StatusCode::OK);
    let import_body: serde_json::Value =
        serde_json::from_slice(&res_import.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert_eq!(import_body["npub"], npub);

    let req_invalid = Request::builder()
        .method("POST")
        .uri("/api/identity/import")
        .header("content-type", "application/json")
        .header("x-requested-with", "mostro-community")
        .header("host", "localhost:5173")
        .body(Body::from(
            serde_json::json!({"nsec": "nsec1invalid"}).to_string(),
        ))
        .unwrap();
    let res_invalid = app2.oneshot(req_invalid).await.unwrap();
    assert_eq!(res_invalid.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn community_card_endpoint_contract() {
    let (dir, app) = setup();
    let get_card = |app: axum::Router| async move {
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/community/card")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        (status, body)
    };

    // Without identity or saved configuration there is nothing to sign. A
    // half card (null pubkey or signature) would not even parse in the app.
    // The answer says which of the two is missing.
    let (status, body) = get_card(app.clone()).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["error"].as_str().unwrap().contains("reglas guardadas"));

    let response = app
        .clone()
        .oneshot(save(0, true, "http://localhost:5173", config()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let (status, body) = get_card(app.clone()).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["error"].as_str().unwrap().contains("identidad"));

    use nostr::{Keys, SecretKey, ToBech32};
    let keys = Keys::new(SecretKey::from_slice(&[21; 32]).unwrap());
    mostro_community_api::identity::import(
        dir.path(),
        &keys.secret_key().to_bech32().unwrap(),
        &keys.public_key().to_bech32().unwrap(),
    )
    .unwrap();

    let (status, body) = get_card(app.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["pubkey"].is_string());
    assert!(body["signature"].is_string());
    let card: mostro_community_api::connection::CommunityCard =
        serde_json::from_value(body).unwrap();
    assert_eq!(card.version, 1);
    assert_eq!(card.pubkey, keys.public_key().to_hex());
    assert_eq!(card.currency, "EUR");
    assert!(mostro_community_api::connection::verify_community_card(
        &card
    ));
}

/// A draft saved by an earlier version may hold a separator the signed card
/// cannot carry. The answer says so: nothing is missing, and saving again
/// without changing it would be refused.
#[tokio::test]
async fn community_card_endpoint_says_why_there_is_no_card() {
    let get_card = |app: axum::Router| async move {
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/community/card")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        (
            status,
            body["error"].as_str().unwrap_or_default().to_string(),
        )
    };
    let import_identity = |root: &std::path::Path| {
        use nostr::{Keys, SecretKey, ToBech32};
        let keys = Keys::new(SecretKey::from_slice(&[22; 32]).unwrap());
        mostro_community_api::identity::import(
            root,
            &keys.secret_key().to_bech32().unwrap(),
            &keys.public_key().to_bech32().unwrap(),
        )
        .unwrap();
    };

    // An identity and no rules: the rules are what is missing.
    let dir = tempfile::tempdir().unwrap();
    import_identity(dir.path());
    let (status, reason) = get_card(app_for(dir.path())).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(reason.contains("Aún no hay reglas guardadas"), "{reason}");
    assert!(!reason.contains("identidad"), "{reason}");

    // An identity and rules as v1.0.11 accepted them.
    let dir = tempfile::tempdir().unwrap();
    import_identity(dir.path());
    let mut earlier = config();
    earlier["community"]["name"] = "Compra & venta".into();
    std::fs::write(
        dir.path().join("community.json"),
        serde_json::json!({"revision": 4, "config": earlier}).to_string(),
    )
    .unwrap();
    let app = app_for(dir.path());
    let (status, reason) = get_card(app.clone()).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        reason.contains("El nombre de la comunidad no puede contener «&»"),
        "{reason}"
    );
    assert!(!reason.contains("identidad"), "{reason}");
    assert!(!reason.contains("no hay reglas"), "{reason}");

    // The connection data gives the panel the same reason.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/connection")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let connection: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(connection["status"], "ready");
    assert!(connection.get("card").is_none());
    assert_eq!(connection["card_unavailable"], reason);

    // Saving it again as it is, is refused with the same rule.
    let response = app
        .clone()
        .oneshot(save(4, true, "http://localhost:5173", earlier.clone()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(reason.contains(body["error"].as_str().unwrap()), "{body}");

    // Once the name is changed the card is back, with no reason to give.
    earlier["community"]["name"] = "Compra y venta".into();
    let response = app
        .clone()
        .oneshot(save(4, true, "http://localhost:5173", earlier))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let (status, _) = get_card(app.clone()).await;
    assert_eq!(status, StatusCode::OK);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/connection")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let connection: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(connection["card"].is_object());
    assert!(connection.get("card_unavailable").is_none());
}

/// `/api/dashboard` and `/api/daemon/status` answer for the same node. After
/// a restart neither takes what the previous process announced for the new one.
#[tokio::test]
async fn dashboard_and_status_agree_on_an_announcement_from_before_the_restart() {
    use nostr::{Keys, SecretKey, ToBech32};
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    let dir = tempfile::tempdir().unwrap();
    // Activation only writes under a private CONFIG_DIR.
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let keys = Keys::new(SecretKey::from_slice(&[23; 32]).unwrap());
    mostro_community_api::identity::import(
        dir.path(),
        &keys.secret_key().to_bech32().unwrap(),
        &keys.public_key().to_bech32().unwrap(),
    )
    .unwrap();
    let (app, orders) = app_with_orders(dir.path());
    let response = app
        .clone()
        .oneshot(save(0, true, "http://localhost:5173", config()))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    mostro_community_api::daemon::activate(dir.path(), "https://127.0.0.1:10009").unwrap();

    // A daemon the supervisor has just started, and on the relays what the
    // one before it announced three minutes ago.
    let active = dir.path().join("active");
    std::fs::write(active.join("mostro.pid"), "4242\n").unwrap();
    std::fs::write(active.join("mostro.heartbeat"), "").unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    orders.write().await.node_info = Some(mostro_community_api::orders::NodeInfo {
        event_id: "b".repeat(64),
        created_at: now - 180,
        name: None,
        mostro_version: Some("0.17.5".into()),
        protocol_version: Some("2".into()),
        tags: Default::default(),
    });
    let get = |app: axum::Router, uri: &'static str| async move {
        let response = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        serde_json::from_slice::<serde_json::Value>(
            &response.into_body().collect().await.unwrap().to_bytes(),
        )
        .unwrap()
    };

    let status = get(app.clone(), "/api/daemon/status").await;
    assert_eq!(status["state"], "active_running");
    assert_eq!(status["announced_fresh"], false);
    assert_eq!(status["announced_before_start"], true);
    let dashboard = get(app.clone(), "/api/dashboard").await;
    assert_eq!(dashboard["market_started"], false, "{dashboard}");
    assert_eq!(dashboard["mostro"]["status"], "unknown");
    assert!(dashboard["mostro"].get("version").is_none(), "{dashboard}");
    assert!(
        dashboard["mostro"]["detail"]
            .as_str()
            .unwrap()
            .contains("anterior a este arranque"),
        "{dashboard}"
    );

    // The same announcement from a daemon that was already running counts,
    // in both answers.
    std::fs::File::options()
        .write(true)
        .open(active.join("mostro.pid"))
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(600))
        .unwrap();
    let status = get(app.clone(), "/api/daemon/status").await;
    assert_eq!(status["announced_fresh"], true);
    let dashboard = get(app, "/api/dashboard").await;
    assert_eq!(dashboard["market_started"], true, "{dashboard}");
    assert_eq!(dashboard["mostro"]["status"], "online");
    assert_eq!(dashboard["mostro"]["version"], "0.17.5");
}

#[tokio::test]
async fn saving_rejects_a_dev_fee_mostrod_would_refuse() {
    let (_dir, app) = setup();
    let mut low = config();
    low["market"]["dev_fee_bps"] = 500.into();
    let response = app
        .clone()
        .oneshot(save(0, true, "http://localhost:5173", low))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(body["error"].as_str().unwrap().contains("10 %"));

    let mut minimum = config();
    minimum["market"]["dev_fee_bps"] = 1000.into();
    let response = app
        .oneshot(save(0, true, "http://localhost:5173", minimum))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn disputes_endpoint_requires_protection() {
    let (_dir, app) = setup();
    let request = |protected: bool| {
        let mut builder = Request::builder().uri("/api/disputes");
        if protected {
            builder = builder.header("x-requested-with", "mostro-community");
        }
        builder.body(Body::empty()).unwrap()
    };
    let response = app.clone().oneshot(request(false)).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = app.oneshot(request(true)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body, serde_json::json!([]));
}

#[test]
fn the_rate_limit_counts_the_connecting_address_not_a_header() {
    use axum::http::{HeaderMap, HeaderValue};
    use mostro_community_api::{client_key, parse_proxies};
    use std::net::IpAddr;

    let forged = |value: &str| {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_str(value).unwrap());
        headers
    };
    let client: IpAddr = "203.0.113.7".parse().unwrap();
    let proxy: IpAddr = "172.18.0.5".parse().unwrap();
    let trusted = parse_proxies("172.16.0.0/12, ::1, nonsense, 10.0.0.0/99");
    assert_eq!(trusted.len(), 2, "invalid entries are skipped");

    // No proxy in front: a forged header does not move the client to another bucket.
    assert_eq!(
        client_key(Some(client), &forged("1.2.3.4"), &trusted),
        "203.0.113.7"
    );
    assert_eq!(
        client_key(Some(client), &forged("1.2.3.4"), &[]),
        "203.0.113.7"
    );
    // Behind a trusted proxy: the nearest address that is not a proxy.
    assert_eq!(
        client_key(
            Some(proxy),
            &forged("1.2.3.4, 198.51.100.9, 172.18.0.2"),
            &trusted
        ),
        "198.51.100.9"
    );
    // A trusted proxy without the header counts as itself.
    assert_eq!(
        client_key(Some(proxy), &HeaderMap::new(), &trusted),
        "172.18.0.5"
    );
    // Requests without a connection address (tests, in-process calls).
    assert_eq!(client_key(None, &forged("1.2.3.4"), &trusted), "local");
}
