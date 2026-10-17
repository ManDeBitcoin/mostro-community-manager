use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use mostro_community_api::{
    AppState,
    adapters::Integrations,
    auth::{AuthMode, PanelAuth},
    router,
    store::Store,
};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

const PASSWORD: &str = "correct horse battery";

fn app(root: &std::path::Path, auth: PanelAuth) -> axum::Router {
    let initial_config: mostro_community_api::config::Configuration =
        serde_json::from_str(include_str!("fixtures/community.json")).unwrap();
    let (tx, _rx) = tokio::sync::watch::channel(mostro_community_api::orders::MonitorCommand {
        config: initial_config,
        npub: None,
    });
    let mut state = AppState::new(
        Arc::new(Mutex::new(Store::open(root.into()).unwrap())),
        Integrations::default(),
        Arc::new(tokio::sync::RwLock::new(
            mostro_community_api::orders::OrdersCache::new(),
        )),
        Arc::new(tokio::sync::RwLock::new(
            mostro_community_api::chat::ChatCache::new(),
        )),
        tx,
    );
    state.auth = Arc::new(auth);
    router(state)
}

fn get(path: &str, cookie: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().uri(path);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    builder.body(Body::empty()).unwrap()
}

fn post(path: &str, body: serde_json::Value, cookie: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .header("host", "panel.example")
        .header("origin", "https://panel.example")
        .header("x-requested-with", "mostro-community");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    builder.body(Body::from(body.to_string())).unwrap()
}

async fn json(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

#[tokio::test]
async fn with_a_password_the_api_asks_for_a_session() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path(), PanelAuth::with_password(PASSWORD).unwrap());

    // What the login page needs answers without a session.
    let health = app.clone().oneshot(get("/api/health", None)).await.unwrap();
    assert_eq!(health.status(), StatusCode::OK);
    assert_eq!(json(health).await["auth"], "password");
    let session = json(
        app.clone()
            .oneshot(get("/api/auth/session", None))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        session,
        serde_json::json!({"mode": "password", "authenticated": false})
    );

    // Everything else does not, unknown paths included.
    for path in [
        "/api/community",
        "/api/dashboard",
        "/api/notifications/sse",
        "/api/nothing",
    ] {
        let response = app.clone().oneshot(get(path, None)).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
        assert_eq!(json(response).await["login_required"], true);
    }
    let forged = app
        .clone()
        .oneshot(get("/api/community", Some("mcm_session=0123456789abcdef")))
        .await
        .unwrap();
    assert_eq!(forged.status(), StatusCode::UNAUTHORIZED);

    let wrong = app
        .clone()
        .oneshot(post(
            "/api/auth/login",
            serde_json::json!({"password": "guess"}),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    assert!(wrong.headers().get(header::SET_COOKIE).is_none());

    let login = app
        .clone()
        .oneshot(post(
            "/api/auth/login",
            serde_json::json!({"password": PASSWORD}),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::OK);
    let set_cookie = login.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .to_string();
    for attribute in [
        "HttpOnly",
        "SameSite=Strict",
        "Path=/",
        "Max-Age=604800",
        "Secure",
    ] {
        assert!(
            set_cookie.contains(attribute),
            "{set_cookie} lacks {attribute}"
        );
    }
    let cookie = set_cookie.split(';').next().unwrap().to_string();

    let community = app
        .clone()
        .oneshot(get("/api/community", Some(&cookie)))
        .await
        .unwrap();
    assert_eq!(community.status(), StatusCode::OK);
    let session = json(
        app.clone()
            .oneshot(get("/api/auth/session", Some(&cookie)))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(session["authenticated"], true);

    let logout = app
        .clone()
        .oneshot(post(
            "/api/auth/logout",
            serde_json::json!({}),
            Some(&cookie),
        ))
        .await
        .unwrap();
    assert_eq!(logout.status(), StatusCode::OK);
    assert!(
        logout.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    let after = app
        .clone()
        .oneshot(get("/api/community", Some(&cookie)))
        .await
        .unwrap();
    assert_eq!(
        after.status(),
        StatusCode::UNAUTHORIZED,
        "the session ends with the logout"
    );
}

#[tokio::test]
async fn the_login_needs_the_panel_header_and_limits_attempts() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(dir.path(), PanelAuth::with_password(PASSWORD).unwrap());

    // Another site cannot log the browser in: the same protection as any change.
    let cross_site = Request::builder()
        .method("POST")
        .uri("/api/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({"password": PASSWORD}).to_string(),
        ))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(cross_site).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );

    for _ in 0..5 {
        let response = app
            .clone()
            .oneshot(post(
                "/api/auth/login",
                serde_json::json!({"password": "guess"}),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    // The sixth attempt within the minute is refused, even with the right password.
    let blocked = app
        .clone()
        .oneshot(post(
            "/api/auth/login",
            serde_json::json!({"password": PASSWORD}),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(blocked.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn without_a_password_nothing_asks_for_a_login() {
    let dir = tempfile::tempdir().unwrap();
    for (auth, mode) in [
        (PanelAuth::external(), "external"),
        (PanelAuth::open(), "open"),
    ] {
        let app = app(dir.path(), auth);
        assert_eq!(
            app.clone()
                .oneshot(get("/api/community", None))
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        let session = json(
            app.clone()
                .oneshot(get("/api/auth/session", None))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(
            session,
            serde_json::json!({"mode": mode, "authenticated": true})
        );
        let login = app
            .clone()
            .oneshot(post(
                "/api/auth/login",
                serde_json::json!({"password": PASSWORD}),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(login.status(), StatusCode::BAD_REQUEST);
    }
}

#[test]
fn the_access_mode_comes_from_the_environment_and_never_leaves_the_panel_open() {
    let mode = |auth: Option<&str>, file: Option<&str>, bind: &str| {
        PanelAuth::from_settings(auth, file, bind).map(|a| a.mode())
    };
    assert_eq!(mode(None, None, "127.0.0.1:3001"), Ok(AuthMode::Open));
    assert_eq!(mode(None, None, "[::1]:3001"), Ok(AuthMode::Open));
    assert_eq!(
        mode(Some(""), Some(""), "localhost:3001"),
        Ok(AuthMode::Open)
    );
    let open = mode(None, None, "0.0.0.0:3001").unwrap_err();
    assert!(
        open.contains("PANEL_PASSWORD_FILE") && open.contains("PANEL_AUTH=external"),
        "{open}"
    );
    assert_eq!(
        mode(Some("external"), None, "0.0.0.0:3001"),
        Ok(AuthMode::External)
    );
    assert!(mode(Some("none"), None, "0.0.0.0:3001").is_err());

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("panel_password");
    std::fs::write(&file, format!("{PASSWORD}\n")).unwrap();
    let file = file.to_str().unwrap();
    assert!(
        mode(Some("external"), Some(file), "0.0.0.0:3001").is_err(),
        "both is ambiguous"
    );
    let auth = PanelAuth::from_settings(None, Some(file), "0.0.0.0:3001").unwrap();
    assert_eq!(auth.mode(), AuthMode::Password);
    // The newline that ends the file is not part of the password.
    assert!(auth.login("a", PASSWORD).is_ok());
    assert!(auth.login("b", &format!("{PASSWORD}\n")).is_err());

    let short = dir.path().join("short");
    std::fs::write(&short, "short\n").unwrap();
    assert!(mode(None, Some(short.to_str().unwrap()), "0.0.0.0:3001").is_err());
    assert!(mode(None, Some("/nonexistent/panel_password"), "0.0.0.0:3001").is_err());
}
