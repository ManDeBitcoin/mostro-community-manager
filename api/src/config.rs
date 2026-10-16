use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Component, Path},
};
use url::Url;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub community: Community,
    pub market: Market,
    pub safety: Safety,
    pub nostr: Nostr,
    pub payment_methods: Vec<PaymentMethod>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Community {
    pub name: String,
    pub about: String,
    pub website: String,
    pub contact: String,
    pub language: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Market {
    pub fiat_currencies: Vec<String>,
    pub min_trade_sats: u64,
    pub max_trade_sats: u64,
    pub fee_bps: u16,
    pub dev_fee_bps: u16,
    pub max_routing_fee_bps: u16,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Safety {
    pub bond_enabled: bool,
    pub bond_bps: u16,
    pub base_bond_sats: u64,
    pub bond_apply_to: BondApply,
    pub automatic_timeout_slash: bool,
    pub pow: u8,
    pub pow_first_contact: u8,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BondApply {
    Make,
    Take,
    Both,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Nostr {
    pub relays: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaymentMethod {
    pub id: String,
    pub label: String,
    pub category: String,
    pub active: bool,
}

fn valid_url(value: &str, schemes: &[&str]) -> bool {
    Url::parse(value).is_ok_and(|u| {
        schemes.contains(&u.scheme())
            && u.host_str().is_some()
            && u.username().is_empty()
            && u.password().is_none()
            && u.fragment().is_none()
    })
}
impl Configuration {
    pub fn validate(&self) -> Result<(), &'static str> {
        let c = &self.community;
        if c.name.trim().is_empty()
            || c.name.len() > 100
            || c.about.len() > 1000
            || c.language.trim().is_empty()
            || c.language.len() > 35
        {
            return Err("Nombre, descripción o idioma no válidos");
        }
        if [&c.website, &c.contact]
            .iter()
            .any(|v| v.len() > 2048 || (!v.is_empty() && !valid_url(v, &["https", "http"])))
        {
            return Err("Usa enlaces HTTP(S) sin credenciales");
        }
        let m = &self.market;
        if m.min_trade_sats == 0
            || m.min_trade_sats > m.max_trade_sats
            || m.max_trade_sats > 100_000_000
        {
            return Err("Límites inválidos: 1 ≤ mínimo ≤ máximo ≤ 100 millones de sats");
        }
        if m.fee_bps > 10_000 || m.dev_fee_bps > 10_000 || m.max_routing_fee_bps > 10_000 {
            return Err("Los porcentajes deben estar entre 0 y 100");
        }
        let mut currencies = HashSet::new();
        if m.fiat_currencies.is_empty()
            || m.fiat_currencies.len() > 30
            || m.fiat_currencies.iter().any(|v| {
                v.len() != 3 || !v.bytes().all(|b| b.is_ascii_uppercase()) || !currencies.insert(v)
            })
        {
            return Err("Selecciona códigos de moneda de tres letras, únicos y en mayúsculas");
        }
        let s = &self.safety;
        if s.bond_bps > 10_000 || s.base_bond_sats > 100_000_000 {
            return Err(
                "La garantía debe estar entre 0 y 100 % y su mínimo no superar 100 millones de sats",
            );
        }
        if s.pow > 32 || s.pow_first_contact > 32 {
            return Err("La prueba de trabajo debe estar entre 0 y 32");
        }
        if s.pow_first_contact < s.pow {
            return Err(
                "La prueba de trabajo de la primera conversación no puede ser menor que la general",
            );
        }
        if s.bond_enabled && s.bond_bps == 0 && s.base_bond_sats == 0 {
            return Err(
                "Con la garantía activada, el porcentaje o el mínimo deben ser mayores que cero",
            );
        }
        let mut relays = HashSet::new();
        if self.nostr.relays.is_empty()
            || self.nostr.relays.len() > 10
            || self
                .nostr
                .relays
                .iter()
                .any(|v| v.len() > 2048 || !valid_url(v, &["ws", "wss"]) || !relays.insert(v))
        {
            return Err("Configura entre 1 y 10 relays WS(S) únicos, sin credenciales");
        }
        let mut ids = HashSet::new();
        if self.payment_methods.len() > 100
            || self.payment_methods.iter().any(|p| {
                p.id.is_empty()
                    || p.id.len() > 64
                    || !p
                        .id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                    || !ids.insert(&p.id)
                    || p.label.trim().is_empty()
                    || p.label.len() > 100
                    || p.category.len() > 100
            })
        {
            return Err("Catálogo de métodos inválido o con identificadores duplicados");
        }
        Ok(())
    }
}

impl Configuration {
    /// Checks applied when the operator saves a draft, on top of [`validate`].
    /// They are not part of [`validate`] because a stored draft from an older
    /// version must still open; the renderer keeps such drafts loadable by
    /// mostrod, but new saves have to say what the daemon will really do.
    ///
    /// [`validate`]: Configuration::validate
    pub fn validate_for_save(&self) -> Result<(), &'static str> {
        self.validate()?;
        // mostrod refuses to start with dev_fee_percentage outside 0.10..=1.0.
        if self.market.dev_fee_bps < MIN_DEV_FEE_BPS {
            return Err(
                "La comisión de desarrollo de Mostro debe estar entre el 10 % y el 100 % de la comisión del nodo",
            );
        }
        if let Some(rule) = self.card_ambiguity() {
            return Err(rule);
        }
        Ok(())
    }

    /// The v1 community card is signed over `key=value&...` with lists joined
    /// by commas and nothing escaped (the app's verifier defines it). A `&` or
    /// a `,` inside a value would let one signature cover two different cards.
    /// Returns the rule this configuration breaks, as the operator reads it.
    pub fn card_ambiguity(&self) -> Option<&'static str> {
        let c = &self.community;
        let in_list = |value: &str| value.contains('&') || value.contains(',');
        if c.name.contains('&') {
            Some(
                "El nombre de la comunidad no puede contener «&»: la tarjeta firmada dejaría de ser inequívoca",
            )
        } else if c.website.contains('&') || c.contact.contains('&') {
            Some(
                "La web y el contacto no pueden contener «&»: la tarjeta firmada dejaría de ser inequívoca",
            )
        } else if self.nostr.relays.iter().any(|relay| in_list(relay)) {
            Some(
                "Los relays no pueden contener «&» ni «,»: la tarjeta firmada dejaría de ser inequívoca",
            )
        } else if self
            .payment_methods
            .iter()
            .any(|method| method.active && in_list(&method.label))
        {
            Some(
                "Los nombres de los métodos de pago activos no pueden contener «&» ni «,»: la tarjeta firmada dejaría de ser inequívoca",
            )
        } else {
            None
        }
    }

    pub fn card_is_ambiguous(&self) -> bool {
        self.card_ambiguity().is_some()
    }
}

/// Lowest `dev_fee_percentage` mostrod accepts (0.10), in basis points.
pub const MIN_DEV_FEE_BPS: u16 = 1_000;

/// Upstream `settings.tpl.toml` the renderer starts from.
///
/// The embedded file is the v0.19.0 template. For mostrod v0.20.0 it yields
/// the same settings: upstream added commented-out blocks and a `[health]`
/// section whose absence mostrod reads as the same defaults, and
/// `api/tests/configuration.rs` fails if the two ever differ otherwise once
/// parsed. The v0.20.0 copy in `config/upstream` is GPL-3.0-or-later like the
/// rest of that release, so it is kept as a reference and not compiled in.
const SETTINGS_TEMPLATE: &str = include_str!("../../config/upstream/settings.v0.19.0.toml");

/// Render an inert candidate from the pinned upstream template. The imported nsec
/// stays in its own file; Mostro will receive it via MOSTRO_NSEC_PRIVKEY only when
/// a separate, tested daemon launcher exists. RPC remains disabled here.
pub fn render_settings(
    config: &Configuration,
    lnd_host: &str,
    cert: &str,
    macaroon: &str,
) -> Result<String, String> {
    config.validate().map_err(str::to_owned)?;
    let host = Url::parse(lnd_host).map_err(|_| "Host LND inválido")?;
    if host.scheme() != "https"
        || host.host_str().is_none()
        || !host.username().is_empty()
        || host.password().is_some()
        || host.path() != "/"
        || host.query().is_some()
        || host.fragment().is_some()
    {
        return Err("Host LND debe ser un origen HTTPS sin credenciales".into());
    }
    if [cert, macaroon].iter().any(|value| {
        let path = Path::new(value);
        !path.is_absolute()
            || path
                .components()
                .any(|part| matches!(part, Component::ParentDir))
    }) {
        return Err("Las rutas de credenciales LND deben ser absolutas".into());
    }
    let mut doc: toml::Value =
        toml::from_str(SETTINGS_TEMPLATE).map_err(|_| "Plantilla inválida")?;
    doc["lightning"]["lnd_grpc_host"] = lnd_host.into();
    doc["lightning"]["lnd_cert_file"] = cert.into();
    doc["lightning"]["lnd_macaroon_file"] = macaroon.into();
    doc["nostr"]["nsec_privkey"] = "".into();
    doc["nostr"]["relays"] =
        toml::Value::try_from(&config.nostr.relays).map_err(|_| "Relays inválidos")?;
    // Kind 0 metadata. An empty value is left out: mostrod logs
    // "Invalid website URL" on every start for an empty website.
    for (key, value) in [
        ("name", &config.community.name),
        ("about", &config.community.about),
        ("website", &config.community.website),
    ] {
        let value = value.trim();
        if !value.is_empty() {
            doc["mostro"]
                .as_table_mut()
                .unwrap()
                .insert(key.into(), value.into());
        }
    }
    for (key, value) in [
        // Total fee of a trade; mostrod charges half to each party.
        ("fee", config.market.fee_bps),
        // Share of that fee sent to Mostro development. Older drafts may hold
        // less than the minimum mostrod accepts; they are raised to it.
        (
            "dev_fee_percentage",
            config.market.dev_fee_bps.max(MIN_DEV_FEE_BPS),
        ),
        ("max_routing_fee", config.market.max_routing_fee_bps),
    ] {
        doc["mostro"][key] = (f64::from(value) / 10_000.0).into();
    }
    doc["mostro"]["min_payment_amount"] = (config.market.min_trade_sats as i64).into();
    doc["mostro"]["max_order_amount"] = (config.market.max_trade_sats as i64).into();
    doc["mostro"]["fiat_currencies_accepted"] =
        toml::Value::try_from(&config.market.fiat_currencies).map_err(|_| "Monedas inválidas")?;
    doc["mostro"]["pow"] = i64::from(config.safety.pow).into();
    doc["mostro"].as_table_mut().unwrap().insert(
        "pow_first_contact".into(),
        i64::from(config.safety.pow_first_contact).into(),
    );
    doc["rpc"]["enabled"] = false.into();
    doc["rpc"]["listen_address"] = "127.0.0.1".into();
    doc["rpc"].as_table_mut().unwrap().remove("auth_token");
    let bond = toml::toml! {
        enabled = (config.safety.bond_enabled)
        amount_pct = (f64::from(config.safety.bond_bps) / 10_000.0)
        base_amount_sats = (config.safety.base_bond_sats as i64)
        apply_to = (match config.safety.bond_apply_to { BondApply::Make => "make", BondApply::Take => "take", BondApply::Both => "both" })
        slash_on_waiting_timeout = (config.safety.automatic_timeout_slash)
        slash_node_share_pct = 0.5
        payout_invoice_window_seconds = 300
        payout_max_retries = 5
        payout_claim_window_days = 15
        maker_bond_payment_timeout_seconds = 900
    };
    doc.as_table_mut()
        .unwrap()
        .insert("anti_abuse_bond".into(), bond.into());
    toml::to_string_pretty(&doc).map_err(|_| "No se pudo generar TOML".into())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegionalPreset {
    pub id: String,
    pub title: String,
    pub description: String,
    pub icon: String,
    pub config: Configuration,
}

pub fn get_regional_presets() -> Vec<RegionalPreset> {
    vec![
        RegionalPreset {
            id: "latam".into(),
            title: "Latinoamérica (LATAM)".into(),
            description: "Configuración optimizada para mercados en Latam: Pago Móvil, Zelle, Mercado Pago, PIX, etc.".into(),
            icon: "🌎".into(),
            config: Configuration {
                community: Community {
                    name: "Mostro P2P Latam".into(),
                    about: "Comunidad de intercambio P2P de Bitcoin Lightning para Latinoamérica.".into(),
                    website: "https://mostro.network".into(),
                    contact: "https://t.me/MostroP2P".into(),
                    language: "es".into(),
                },
                market: Market {
                    fiat_currencies: vec![
                        "USD".into(), "ARS".into(), "VES".into(), "COP".into(),
                        "BRL".into(), "MXN".into(), "CLP".into(), "PEN".into(),
                    ],
                    min_trade_sats: 10_000,
                    max_trade_sats: 1_000_000,
                    fee_bps: 50,
                    dev_fee_bps: 1_000,
                    max_routing_fee_bps: 10,
                },
                safety: Safety {
                    bond_enabled: true,
                    bond_bps: 300,
                    base_bond_sats: 5_000,
                    bond_apply_to: BondApply::Both,
                    automatic_timeout_slash: true,
                    pow: 0,
                    pow_first_contact: 0,
                },
                nostr: Nostr {
                    relays: vec![
                        "wss://relay.damus.io".into(),
                        "wss://nos.lol".into(),
                        "wss://nostr.mom".into(),
                    ],
                },
                payment_methods: vec![
                    PaymentMethod {
                        id: "pago_movil".into(),
                        label: "Pago Móvil".into(),
                        category: "Venezuela".into(),
                        active: true,
                    },
                    PaymentMethod {
                        id: "transferencia_bancaria".into(),
                        label: "Transferencia Bancaria".into(),
                        category: "Nacional / Regional".into(),
                        active: true,
                    },
                    PaymentMethod {
                        id: "zelle".into(),
                        label: "Zelle".into(),
                        category: "USD Internacional".into(),
                        active: true,
                    },
                    PaymentMethod {
                        id: "mercado_pago".into(),
                        label: "Mercado Pago".into(),
                        category: "Latam Digital".into(),
                        active: true,
                    },
                    PaymentMethod {
                        id: "pix".into(),
                        label: "PIX".into(),
                        category: "Brasil".into(),
                        active: true,
                    },
                    PaymentMethod {
                        id: "nequi_bancolombia".into(),
                        label: "Nequi / Bancolombia".into(),
                        category: "Colombia".into(),
                        active: true,
                    },
                ],
            },
        },
        RegionalPreset {
            id: "europe".into(),
            title: "Europa (SEPA / Bizum)".into(),
            description: "Configuración para zona SEPA, Bizum, Revolut, EUR y CHF.".into(),
            icon: "🇪🇺".into(),
            config: Configuration {
                community: Community {
                    name: "Mostro P2P Europe".into(),
                    about: "Bitcoin Lightning P2P community for Europe & SEPA zone.".into(),
                    website: "https://mostro.network".into(),
                    contact: "https://t.me/MostroP2P".into(),
                    language: "en".into(),
                },
                market: Market {
                    fiat_currencies: vec!["EUR".into(), "CHF".into(), "GBP".into()],
                    min_trade_sats: 20_000,
                    max_trade_sats: 2_000_000,
                    fee_bps: 50,
                    dev_fee_bps: 1_000,
                    max_routing_fee_bps: 10,
                },
                safety: Safety {
                    bond_enabled: true,
                    bond_bps: 300,
                    base_bond_sats: 10_000,
                    bond_apply_to: BondApply::Both,
                    automatic_timeout_slash: true,
                    pow: 0,
                    pow_first_contact: 0,
                },
                nostr: Nostr {
                    relays: vec![
                        "wss://relay.damus.io".into(),
                        "wss://nos.lol".into(),
                        "wss://nostr.mom".into(),
                    ],
                },
                payment_methods: vec![
                    PaymentMethod {
                        id: "bizum".into(),
                        label: "Bizum".into(),
                        category: "España".into(),
                        active: true,
                    },
                    PaymentMethod {
                        id: "sepa_instant".into(),
                        label: "SEPA Instant".into(),
                        category: "Eurozona".into(),
                        active: true,
                    },
                    PaymentMethod {
                        id: "revolut".into(),
                        label: "Revolut".into(),
                        category: "Multi-divisa".into(),
                        active: true,
                    },
                    PaymentMethod {
                        id: "n26".into(),
                        label: "N26".into(),
                        category: "Eurozona".into(),
                        active: true,
                    },
                    PaymentMethod {
                        id: "wise".into(),
                        label: "Wise".into(),
                        category: "Internacional".into(),
                        active: true,
                    },
                ],
            },
        },
        RegionalPreset {
            id: "global".into(),
            title: "P2P Global / Internacional".into(),
            description: "Configuración flexible para operaciones globales con Wise, Revolut y efectivo.".into(),
            icon: "🌐".into(),
            config: Configuration {
                community: Community {
                    name: "Mostro P2P Global".into(),
                    about: "Global peer-to-peer Bitcoin Lightning exchange community.".into(),
                    website: "https://mostro.network".into(),
                    contact: "https://t.me/MostroP2P".into(),
                    language: "en".into(),
                },
                market: Market {
                    fiat_currencies: vec!["USD".into(), "EUR".into()],
                    min_trade_sats: 20_000,
                    max_trade_sats: 2_000_000,
                    fee_bps: 50,
                    dev_fee_bps: 1_000,
                    max_routing_fee_bps: 10,
                },
                safety: Safety {
                    bond_enabled: true,
                    bond_bps: 300,
                    base_bond_sats: 10_000,
                    bond_apply_to: BondApply::Both,
                    automatic_timeout_slash: true,
                    pow: 0,
                    pow_first_contact: 0,
                },
                nostr: Nostr {
                    relays: vec![
                        "wss://relay.damus.io".into(),
                        "wss://nos.lol".into(),
                        "wss://nostr.mom".into(),
                    ],
                },
                payment_methods: vec![
                    PaymentMethod {
                        id: "wise".into(),
                        label: "Wise".into(),
                        category: "Internacional".into(),
                        active: true,
                    },
                    PaymentMethod {
                        id: "revolut".into(),
                        label: "Revolut".into(),
                        category: "Multi-divisa".into(),
                        active: true,
                    },
                    PaymentMethod {
                        id: "swift".into(),
                        label: "SWIFT Transfer".into(),
                        category: "Bancario Internacional".into(),
                        active: true,
                    },
                    PaymentMethod {
                        id: "cash_in_person".into(),
                        label: "Efectivo en Persona (Cash)".into(),
                        category: "Presencial".into(),
                        active: true,
                    },
                ],
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_regional_presets_are_valid() {
        for preset in get_regional_presets() {
            assert!(preset.config.validate().is_ok());
            // A preset must be savable as it is.
            assert!(preset.config.validate_for_save().is_ok());
        }
    }
}
