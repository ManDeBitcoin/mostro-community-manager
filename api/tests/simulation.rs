use mostro_community_api::{
    config::{BondApply, Community, Configuration, Market, Nostr, PaymentMethod, Safety},
    simulation::{self, Actor, SimulationScenario},
};

fn sample_config() -> Configuration {
    Configuration {
        community: Community {
            name: "Comunidad Regtest".into(),
            about: "Comunidad de prueba para simulación P2P".into(),
            website: "https://comunidad.test".into(),
            picture: String::new(),
            contact: "https://t.me/mostro_test".into(),
            language: "es".into(),
        },
        market: Market {
            fiat_currencies: vec!["EUR".into()],
            min_trade_sats: 10_000,
            max_trade_sats: 1_000_000,
            fee_bps: 60, // 0.6%
            dev_fee_bps: 0,
            max_routing_fee_bps: 10,
        },
        safety: Safety {
            bond_enabled: true,
            bond_bps: 300,         // 3.0%
            base_bond_sats: 1_000, // 1000 sats base
            bond_apply_to: BondApply::Both,
            automatic_timeout_slash: true,
            pow: 0,
            pow_first_contact: 0,
        },
        nostr: Nostr {
            relays: vec!["wss://relay.damus.io".into()],
        },
        payment_methods: vec![PaymentMethod {
            id: "sepa".into(),
            label: "SEPA Instant".into(),
            category: "bank".into(),
            active: true,
        }],
    }
}

#[test]
fn happy_path_simulation_calculates_exact_bonds_and_settlement() {
    let config = sample_config();
    let report = simulation::run_simulation(
        &config,
        Some("npub1qqdagara05n9ahlrh5ah9xvgv9r2mpgd2yy4lemmwc7ryq2kskuswt0t3x"),
        SimulationScenario::HappyPath,
        Some(50_000),
    )
    .expect("Simulación falló");

    assert_eq!(report.scenario, SimulationScenario::HappyPath);
    assert_eq!(report.final_status, "success");
    assert!(report.is_success);

    // 50,000 sats * 0.6% = 300 sats fee, half charged to each party
    assert_eq!(report.financials.total_mostro_fee_sats, 300);
    assert_eq!(report.financials.fee_per_side_sats, 150);

    // Bond = max(50,000 * 3.0%, 1000 base) = 1500 sats: the base is a floor
    assert_eq!(report.financials.seller_bond_sats, 1500);
    assert_eq!(report.financials.buyer_bond_sats, 1500);

    // The seller's escrow hold invoice carries the trade plus half the fee;
    // the bond is a second hold invoice.
    assert_eq!(report.financials.seller_hold_invoice_sats, 50_150);
    assert_eq!(report.financials.seller_total_locked_sats, 51_650);

    // The buyer locks only the bond and receives the trade minus half the fee.
    assert_eq!(report.financials.buyer_total_locked_sats, 1500);
    assert_eq!(report.financials.buyer_receives_sats, 49_850);

    let codes: Vec<&str> = report
        .steps
        .iter()
        .map(|s| s.action_code.as_str())
        .collect();
    assert_eq!(
        codes,
        [
            "new_order",
            "maker_bond_accepted",
            "order_published",
            "take_sell",
            "taker_bond_accepted",
            "add_invoice",
            "hold_invoice_created",
            "hold_invoice_accepted",
            "fiat_sent",
            "release",
            "success",
            "maker_bond_released",
            "taker_bond_released",
        ]
    );
    assert!(
        report
            .steps
            .iter()
            .enumerate()
            .all(|(index, step)| step.step_number == index + 1)
    );
    assert_eq!(report.steps.first().unwrap().actor, Actor::Seller);
    assert_eq!(report.steps.last().unwrap().actor, Actor::Mostro);
    assert_eq!(report.steps.last().unwrap().order_status, "success");

    // The order is published before any escrow exists, and the escrow is
    // created only after a buyer took the order.
    let position = |code: &str| codes.iter().position(|c| *c == code).unwrap();
    assert!(position("order_published") < position("take_sell"));
    assert!(position("take_sell") < position("hold_invoice_created"));

    // mostrod v0.19.x speaks protocol v2 only: every private message is kind 14.
    for step in &report.steps {
        if let Some(event) = &step.nostr_event {
            assert!(
                [14, 38383].contains(&event.kind),
                "unexpected kind {} in {}",
                event.kind,
                step.action_code
            );
            assert_eq!(event.recipient.is_some(), event.kind == 14);
        }
    }
    let payout = report
        .steps
        .iter()
        .find(|s| s.action_code == "success")
        .and_then(|s| s.lightning_action.as_ref())
        .unwrap();
    assert_eq!(payout.amount_sats, 49_850);
}

/// Amounts of a trade run against the official v0.19.2 binary on regtest:
/// 56,076 sats at 0.6 % total fee. The seller paid a hold invoice of 56,244
/// sats and the buyer's invoice received 55,908 sats.
#[test]
fn matches_the_amounts_of_a_real_v0_19_2_trade() {
    let mut config = sample_config();
    config.safety.bond_enabled = false;
    let fin = simulation::calculate_financials(&config, 56_076, "USD", "50");
    assert_eq!(fin.fee_per_side_sats, 168);
    assert_eq!(fin.seller_hold_invoice_sats, 56_244);
    assert_eq!(fin.buyer_receives_sats, 55_908);
    assert_eq!(fin.seller_bond_sats, 0);
    assert_eq!(fin.buyer_bond_sats, 0);
    assert_eq!(fin.seller_total_locked_sats, 56_244);
    assert_eq!(fin.buyer_total_locked_sats, 0);

    // Without bonds the walkthrough has no bond steps at all.
    let report =
        simulation::run_simulation(&config, None, SimulationScenario::HappyPath, Some(56_076))
            .unwrap();
    assert!(report.steps.iter().all(|s| !s.action_code.contains("bond")));
    assert_eq!(report.steps.len(), 9);
}

#[test]
fn bond_is_the_larger_of_the_percentage_and_the_floor() {
    let mut config = sample_config();
    // 3 % of 20,000 = 600 sats, below the 1000 sats floor.
    let fin = simulation::calculate_financials(&config, 20_000, "EUR", "12.00");
    assert_eq!(fin.seller_bond_sats, 1000);
    // 3 % of 400,000 = 12,000 sats, above the floor.
    let fin = simulation::calculate_financials(&config, 400_000, "EUR", "240.00");
    assert_eq!(fin.seller_bond_sats, 12_000);

    // The bond applies to the side the node chose.
    config.safety.bond_apply_to = mostro_community_api::config::BondApply::Take;
    let fin = simulation::calculate_financials(&config, 400_000, "EUR", "240.00");
    assert_eq!((fin.seller_bond_sats, fin.buyer_bond_sats), (0, 12_000));
    config.safety.bond_apply_to = mostro_community_api::config::BondApply::Make;
    let fin = simulation::calculate_financials(&config, 400_000, "EUR", "240.00");
    assert_eq!((fin.seller_bond_sats, fin.buyer_bond_sats), (12_000, 0));
}

#[test]
fn dispute_scenarios_execute_solver_resolutions_faithfully() {
    let config = sample_config();

    // Fallo a favor del comprador
    let buyer_favored = simulation::run_simulation(
        &config,
        None,
        SimulationScenario::DisputeSettledForBuyer,
        Some(20_000),
    )
    .expect("Simulación fallo comprador falló");

    assert_eq!(buyer_favored.final_status, "success_dispute_resolved");
    assert!(buyer_favored.steps.iter().any(|s| s.actor == Actor::Solver));
    assert!(
        buyer_favored
            .steps
            .iter()
            .any(|s| s.order_status == "dispute")
    );
    // The dispute is announced on its own public kind and a solver has to
    // take it before resolving it.
    let codes: Vec<&str> = buyer_favored
        .steps
        .iter()
        .map(|s| s.action_code.as_str())
        .collect();
    let position = |code: &str| codes.iter().position(|c| *c == code).unwrap();
    assert!(position("dispute_opened") < position("admin_take_dispute"));
    assert!(position("admin_take_dispute") < position("adm_settle"));
    let opened = &buyer_favored.steps[position("dispute_opened")];
    assert_eq!(opened.nostr_event.as_ref().unwrap().kind, 38386);
    let settle = &buyer_favored.steps[position("adm_settle")];
    assert_eq!(settle.lightning_action.as_ref().unwrap().status, "SETTLED");
    // The buyer still receives the trade minus the fee.
    let payout = &buyer_favored.steps[position("dispute_payout")];
    assert_eq!(
        payout.lightning_action.as_ref().unwrap().amount_sats,
        buyer_favored.financials.buyer_receives_sats
    );

    // Fallo a favor del vendedor
    let seller_favored = simulation::run_simulation(
        &config,
        None,
        SimulationScenario::DisputeRefundedToSeller,
        Some(20_000),
    )
    .expect("Simulación fallo vendedor falló");

    assert_eq!(seller_favored.final_status, "refunded_to_seller");
    let refund = seller_favored
        .steps
        .iter()
        .find(|s| s.action_code == "adm_refund")
        .expect("admin-cancel step");
    assert_eq!(refund.actor, Actor::Solver);
    // The escrow is canceled, not settled: the sats never leave the seller.
    assert_eq!(refund.lightning_action.as_ref().unwrap().status, "CANCELED");
    assert_eq!(
        refund.lightning_action.as_ref().unwrap().amount_sats,
        seller_favored.financials.seller_hold_invoice_sats
    );
    // No payout to the buyer in this outcome.
    assert!(
        seller_favored
            .steps
            .iter()
            .filter_map(|s| s.lightning_action.as_ref())
            .all(|l| l.action != "PayoutToBuyerDispatched")
    );
    let slashed = seller_favored
        .steps
        .iter()
        .find(|s| s.action_code == "buyer_bond_slashed")
        .expect("the buyer's bond is slashed");
    assert_eq!(slashed.lightning_action.as_ref().unwrap().status, "SETTLED");
}

#[test]
fn seller_cancellation_cancels_hold_invoice_without_penalties() {
    let config = sample_config();
    let report = simulation::run_simulation(
        &config,
        None,
        SimulationScenario::SellerCancellation,
        Some(30_000),
    )
    .expect("Simulación cancelación falló");

    assert_eq!(report.final_status, "canceled");
    // new-order, maker bond, publication, cancel: no escrow was ever created.
    assert_eq!(report.steps.len(), 4);
    let cancel_step = report.steps.last().unwrap();
    assert_eq!(cancel_step.order_status, "canceled");
    let l_act = cancel_step.lightning_action.as_ref().unwrap();
    assert_eq!(l_act.action, "MakerBondHoldInvoiceCanceled");
    assert_eq!(l_act.status, "CANCELED");
    assert_eq!(l_act.amount_sats, report.financials.seller_bond_sats);

    // Without a maker bond a pending order has nothing locked at all.
    let mut config = sample_config();
    config.safety.bond_enabled = false;
    let report = simulation::run_simulation(
        &config,
        None,
        SimulationScenario::SellerCancellation,
        Some(30_000),
    )
    .unwrap();
    assert_eq!(report.steps.len(), 3);
    assert!(report.steps.iter().all(|s| s.lightning_action.is_none()));
}

#[test]
fn respects_custom_trade_amount_and_rejects_out_of_bounds() {
    let config = sample_config();

    // Menor al mínimo (10_000)
    let too_low =
        simulation::run_simulation(&config, None, SimulationScenario::HappyPath, Some(5_000));
    assert!(too_low.is_err());

    // Mayor al máximo (1_000_000)
    let too_high = simulation::run_simulation(
        &config,
        None,
        SimulationScenario::HappyPath,
        Some(2_000_000),
    );
    assert!(too_high.is_err());

    // Cero sats
    let zero_sats =
        simulation::run_simulation(&config, None, SimulationScenario::HappyPath, Some(0));
    assert!(zero_sats.is_err());
}

#[test]
fn cli_simulation_strict_argument_parsing_and_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = mostro_community_api::store::Store::open(dir.path().into()).unwrap();
    store.save(sample_config()).unwrap();

    // 1. Sin argumentos: default happy-path con monto clamped válido
    let res = simulation::run_cli_simulation(dir.path(), &[]).unwrap();
    assert_eq!(res.scenario, SimulationScenario::HappyPath);
    assert_eq!(res.financials.trade_amount_sats, 50_000);
    assert_eq!(res.simulation_mode, "synthetic_dry_run");
    assert!(res.disclaimer.contains("sintética"));

    // 2. Escenario explícito soportado
    let res = simulation::run_cli_simulation(dir.path(), &["cancel".into()]).unwrap();
    assert_eq!(res.scenario, SimulationScenario::SellerCancellation);

    let res = simulation::run_cli_simulation(dir.path(), &["dispute-buyer".into(), "25000".into()])
        .unwrap();
    assert_eq!(res.scenario, SimulationScenario::DisputeSettledForBuyer);
    assert_eq!(res.financials.trade_amount_sats, 25_000);

    // 3. Escenario inválido -> Error (NO sustituido silenciosamente por default)
    let err =
        simulation::run_cli_simulation(dir.path(), &["invented_scenario".into()]).unwrap_err();
    assert!(err.contains("Escenario de simulación no válido"));

    // 4. Monto negativo -> Error
    let err = simulation::run_cli_simulation(dir.path(), &["happy-path".into(), "-500".into()])
        .unwrap_err();
    assert!(err.contains("Monto de intercambio inválido"));

    // 5. Monto decimal -> Error
    let err = simulation::run_cli_simulation(dir.path(), &["happy-path".into(), "50000.5".into()])
        .unwrap_err();
    assert!(err.contains("Monto de intercambio inválido"));

    // 6. Monto cero -> Error
    let err =
        simulation::run_cli_simulation(dir.path(), &["happy-path".into(), "0".into()]).unwrap_err();
    assert!(err.contains("Monto de intercambio inválido"));

    // 7. Monto no numérico -> Error
    let err = simulation::run_cli_simulation(dir.path(), &["happy-path".into(), "abc".into()])
        .unwrap_err();
    assert!(err.contains("Monto de intercambio inválido"));

    // 8. Monto con overflow -> Error
    let err = simulation::run_cli_simulation(
        dir.path(),
        &["happy-path".into(), "99999999999999999999999999".into()],
    )
    .unwrap_err();
    assert!(err.contains("Monto de intercambio inválido"));

    // 9. Monto fuera de límites de comunidad (menor a 10_000 o mayor a 1_000_000)
    let err = simulation::run_cli_simulation(dir.path(), &["happy-path".into(), "5000".into()])
        .unwrap_err();
    assert!(err.contains("fuera de los límites de la comunidad"));

    let err = simulation::run_cli_simulation(dir.path(), &["happy-path".into(), "5000000".into()])
        .unwrap_err();
    assert!(err.contains("fuera de los límites de la comunidad"));

    // 10. Argumentos sobrantes -> Error
    let err = simulation::run_cli_simulation(
        dir.path(),
        &["happy-path".into(), "50000".into(), "extra".into()],
    )
    .unwrap_err();
    assert!(err.contains("Demasiados argumentos para simulate-trade"));

    // 11. Sin configuración previa -> Error
    let empty_dir = tempfile::tempdir().unwrap();
    let err = simulation::run_cli_simulation(empty_dir.path(), &[]).unwrap_err();
    assert!(err.contains("Falta configurar la comunidad antes de simular"));
}

#[test]
fn simulation_isolation_never_reads_or_mutates_identity_and_config_files() {
    use nostr::{Keys, SecretKey, ToBech32};
    use sha2::{Digest, Sha256};
    use std::fs;

    let dir = tempfile::tempdir().unwrap();

    // 1. Guardar configuración de comunidad
    let mut store = mostro_community_api::store::Store::open(dir.path().into()).unwrap();
    store.save(sample_config()).unwrap();
    let config_path = dir.path().join("community.json");
    let config_before_bytes = fs::read(&config_path).unwrap();
    let config_hash_before = format!("{:x}", Sha256::digest(&config_before_bytes));

    // 2. Importar una identidad sintética secreta
    let keys = Keys::new(SecretKey::from_slice(&[42; 32]).unwrap());
    let secret_nsec = keys.secret_key().to_bech32().unwrap();
    let real_npub = keys.public_key().to_bech32().unwrap();
    mostro_community_api::identity::import(dir.path(), &secret_nsec, &real_npub).unwrap();

    let identity_path = dir.path().join("identity").join("mostro.nsec");
    let identity_before_bytes = fs::read(&identity_path).unwrap();
    let identity_hash_before = format!("{:x}", Sha256::digest(&identity_before_bytes));
    let identity_meta_before = fs::metadata(&identity_path).unwrap();

    // 3. Ejecutar simulación
    let report = simulation::run_cli_simulation(dir.path(), &["happy-path".into(), "50000".into()])
        .expect("Simulación CLI falló");

    // 4. Verificar que la identidad usada por la simulación es SINTÉTICA y NO la real
    assert_ne!(report.bot_npub, real_npub);
    assert_eq!(report.bot_npub, simulation::SYNTHETIC_BOT_NPUB);

    // 5. Verificar que los ficheros de identidad y configuración permanecen 100% intactos
    let config_after_bytes = fs::read(&config_path).unwrap();
    let config_hash_after = format!("{:x}", Sha256::digest(&config_after_bytes));
    assert_eq!(
        config_hash_before, config_hash_after,
        "community.json fue modificado!"
    );

    let identity_after_bytes = fs::read(&identity_path).unwrap();
    let identity_hash_after = format!("{:x}", Sha256::digest(&identity_after_bytes));
    assert_eq!(
        identity_hash_before, identity_hash_after,
        "mostro.nsec fue alterado!"
    );

    let identity_meta_after = fs::metadata(&identity_path).unwrap();
    assert_eq!(
        identity_meta_before.permissions(),
        identity_meta_after.permissions()
    );
}

#[test]
fn fee_calculation_matches_upstream_rounding_and_edge_cases() {
    let mut config = sample_config();

    // Caso 1: 10,000 sats con 25 bps (0.25%) -> 12.5 sats por lado -> redondeo 13 sats, total real 26 sats
    config.market.fee_bps = 25;
    config.market.dev_fee_bps = 1000; // 10%
    let fin1 = simulation::calculate_financials(&config, 10_000, "EUR", "6.00");
    assert_eq!(fin1.fee_per_side_sats, 13);
    assert_eq!(fin1.total_mostro_fee_sats, 26);
    assert_eq!(fin1.fee_sats, 26);
    // Dev fee: 26 * 10% = 2.6 -> round = 3
    assert_eq!(fin1.dev_fee_sats, 3);

    // Caso 2: Medio satoshi exacto: 1,000 sats con 30 bps (0.30%) -> 1.5 sats por lado -> round 2 sats, total 4 sats
    config.market.fee_bps = 30;
    let fin2 = simulation::calculate_financials(&config, 1_000, "EUR", "0.60");
    assert_eq!(fin2.fee_per_side_sats, 2);
    assert_eq!(fin2.total_mostro_fee_sats, 4);

    // Caso 3: Tarifa cero: 0 bps
    config.market.fee_bps = 0;
    config.market.dev_fee_bps = 0;
    let fin3 = simulation::calculate_financials(&config, 50_000, "EUR", "30.00");
    assert_eq!(fin3.fee_per_side_sats, 0);
    assert_eq!(fin3.total_mostro_fee_sats, 0);
    assert_eq!(fin3.dev_fee_sats, 0);

    // Caso 4: Límite máximo admitido (100 millones de sats con 100 bps = 1%)
    config.market.fee_bps = 100;
    config.market.dev_fee_bps = 3000; // 30%
    let fin4 = simulation::calculate_financials(&config, 100_000_000, "EUR", "60000.00");
    // 100,000,000 * 0.01 / 2 = 500,000 sats por lado
    assert_eq!(fin4.fee_per_side_sats, 500_000);
    assert_eq!(fin4.total_mostro_fee_sats, 1_000_000);
    // Dev fee: 1,000,000 * 30% = 300,000
    assert_eq!(fin4.dev_fee_sats, 300_000);

    // Caso 5: un borrador antiguo con menos del 10 % se simula con el mínimo
    // que mostrod acepta, que es lo que el renderer escribe en settings.toml.
    config.market.dev_fee_bps = 500; // 5 % guardado
    let fin5 = simulation::calculate_financials(&config, 100_000_000, "EUR", "60000.00");
    assert_eq!(fin5.dev_fee_sats, 100_000);

    // Verificación de resolución explícita de hold invoices en todos los escenarios
    let scenarios = [
        SimulationScenario::HappyPath,
        SimulationScenario::DisputeSettledForBuyer,
        SimulationScenario::DisputeRefundedToSeller,
        SimulationScenario::SellerCancellation,
    ];
    for sc in scenarios {
        let rep = simulation::run_simulation(&config, None, sc, Some(50_000)).unwrap();
        // Todas las acciones de hold invoice aceptadas deben tener una resolución final
        let mut accepted_hashes = std::collections::HashSet::new();
        let mut resolved_hashes = std::collections::HashSet::new();
        for st in &rep.steps {
            if let Some(la) = &st.lightning_action {
                if la.status == "ACCEPTED" {
                    accepted_hashes.insert(la.payment_hash.clone());
                } else if la.status == "SETTLED" || la.status == "CANCELED" {
                    resolved_hashes.insert(la.payment_hash.clone());
                }
            }
        }
        for hash in &accepted_hashes {
            assert!(
                resolved_hashes.contains(hash),
                "Escenario {:?} dejó una hold invoice aceptada sin resolución final explícita: {}",
                sc,
                hash
            );
        }
    }
}
