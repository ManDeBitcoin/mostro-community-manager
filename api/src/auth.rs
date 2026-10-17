//! Who may use the panel. The mode is chosen when the API starts:
//! - `PANEL_PASSWORD_FILE`: a password, read from that file; a login gives the
//!   browser a session cookie.
//! - `PANEL_AUTH=external`: no login here, because what sits in front of the
//!   panel already asks for one (Umbrel's app_proxy, a VPN, a proxy that
//!   authenticates).
//! - neither: only while the panel listens on a loopback address (development,
//!   an SSH tunnel). Listening anywhere else without one of the two refuses to
//!   start, so a panel that controls a node's funds is never left open.

use crate::http::RateLimiter;
use axum::http::HeaderMap;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

pub const SESSION_COOKIE: &str = "mcm_session";
pub const SESSION_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
pub const MIN_PASSWORD_LEN: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthMode {
    Password,
    External,
    /// No login: the panel only listens on a loopback address.
    Open,
}

impl AuthMode {
    pub fn as_str(self) -> &'static str {
        match self {
            AuthMode::Password => "password",
            AuthMode::External => "external",
            AuthMode::Open => "open",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum LoginError {
    NotRequired,
    TooManyAttempts,
    WrongPassword,
}

pub struct PanelAuth {
    mode: AuthMode,
    /// SHA-256 of the password: comparing digests takes the same time whatever
    /// the guess has in common with the password.
    password: Option<[u8; 32]>,
    /// Session token -> when it expires.
    sessions: Mutex<HashMap<String, Instant>>,
    attempts: RateLimiter,
}

impl PanelAuth {
    fn new(mode: AuthMode, password: Option<[u8; 32]>) -> Self {
        Self {
            mode,
            password,
            sessions: Mutex::new(HashMap::new()),
            // 5 attempts per minute for each client.
            attempts: RateLimiter::new(5.0, 5.0 / 60.0),
        }
    }

    pub fn open() -> Self {
        Self::new(AuthMode::Open, None)
    }

    pub fn external() -> Self {
        Self::new(AuthMode::External, None)
    }

    pub fn with_password(password: &str) -> Result<Self, String> {
        if password.chars().count() < MIN_PASSWORD_LEN {
            return Err(format!(
                "La contraseña del panel tiene que tener al menos {MIN_PASSWORD_LEN} caracteres."
            ));
        }
        Ok(Self::new(
            AuthMode::Password,
            Some(Sha256::digest(password.as_bytes()).into()),
        ))
    }

    /// The mode for `PANEL_AUTH`, `PANEL_PASSWORD_FILE` and the address the
    /// panel listens on; an error says what to set.
    pub fn from_settings(
        auth: Option<&str>,
        password_file: Option<&str>,
        bind: &str,
    ) -> Result<Self, String> {
        let auth = auth.map(str::trim).filter(|v| !v.is_empty());
        let password_file = password_file.map(str::trim).filter(|v| !v.is_empty());
        match (auth, password_file) {
            (Some("external"), None) => Ok(Self::external()),
            (Some("external"), Some(_)) => Err(
                "Define PANEL_PASSWORD_FILE o PANEL_AUTH=external, no las dos: con una contraseña el panel pide login, con external lo pide otra capa.".into(),
            ),
            (Some(other), _) => Err(format!(
                "PANEL_AUTH={other} no es válido: el único valor es «external»."
            )),
            (None, Some(file)) => {
                let text = std::fs::read_to_string(file).map_err(|e| {
                    format!("No se pudo leer la contraseña del panel en {file}: {e}")
                })?;
                // A file written by `echo` ends in a newline that is not part of the password.
                let password = text.strip_suffix('\n').unwrap_or(&text);
                Self::with_password(password.strip_suffix('\r').unwrap_or(password))
            }
            (None, None) if listens_on_loopback(bind) => Ok(Self::open()),
            (None, None) => Err(format!(
                "El panel escucha en {bind} y no pide contraseña. Define PANEL_PASSWORD_FILE con la ruta de un archivo que contenga la contraseña (al menos {MIN_PASSWORD_LEN} caracteres), o PANEL_AUTH=external si lo que está delante del panel ya pide login (como el app_proxy de Umbrel)."
            )),
        }
    }

    pub fn from_env(bind: &str) -> Result<Self, String> {
        Self::from_settings(
            std::env::var("PANEL_AUTH").ok().as_deref(),
            std::env::var("PANEL_PASSWORD_FILE").ok().as_deref(),
            bind,
        )
    }

    pub fn mode(&self) -> AuthMode {
        self.mode
    }

    /// A new session token for the right password. `client` is who the attempt
    /// counts against.
    pub fn login(&self, client: &str, password: &str) -> Result<String, LoginError> {
        let Some(expected) = self.password else {
            return Err(LoginError::NotRequired);
        };
        if !self.attempts.check(client, 1.0) {
            return Err(LoginError::TooManyAttempts);
        }
        let given: [u8; 32] = Sha256::digest(password.as_bytes()).into();
        let difference = given
            .iter()
            .zip(expected.iter())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b));
        if difference != 0 {
            return Err(LoginError::WrongPassword);
        }
        let token = new_token();
        let now = Instant::now();
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.retain(|_, expires| *expires > now);
            sessions.insert(token.clone(), now + SESSION_TTL);
        }
        Ok(token)
    }

    /// Whether a request may go on: always, unless the panel asks for a password.
    pub fn allows(&self, headers: &HeaderMap) -> bool {
        self.mode != AuthMode::Password
            || session_token(headers).is_some_and(|token| self.is_valid(token))
    }

    fn is_valid(&self, token: &str) -> bool {
        let Ok(mut sessions) = self.sessions.lock() else {
            return false;
        };
        match sessions.get(token) {
            Some(expires) if *expires > Instant::now() => true,
            Some(_) => {
                sessions.remove(token);
                false
            }
            None => false,
        }
    }

    pub fn logout(&self, headers: &HeaderMap) {
        if let (Some(token), Ok(mut sessions)) = (session_token(headers), self.sessions.lock()) {
            sessions.remove(token);
        }
    }
}

fn listens_on_loopback(bind: &str) -> bool {
    match bind.parse::<SocketAddr>() {
        Ok(address) => address.ip().is_loopback(),
        Err(_) => bind.starts_with("localhost:"),
    }
}

fn new_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).expect("the system's random source is unavailable");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The session token the browser sent, if any.
pub fn session_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all("cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .find_map(|pair| pair.trim().strip_prefix(SESSION_COOKIE)?.strip_prefix('='))
        .filter(|token| !token.is_empty())
}

/// `Set-Cookie` for a new session. `secure` when the page was loaded over HTTPS:
/// over plain HTTP (a tunnel, a VPN address) the browser would drop it.
pub fn session_cookie(token: &str, secure: bool) -> String {
    format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{}",
        SESSION_TTL.as_secs(),
        if secure { "; Secure" } else { "" }
    )
}

pub fn expired_cookie() -> String {
    format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0")
}
