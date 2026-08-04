mod assets;
mod challenge;
pub mod data_lab;
mod databank;
mod deploy;
mod discover;
mod evidence;
mod parity_lab;
mod portfolio;
mod promotion_ledger;
mod vault;
mod workflow;

use databank::DesktopState;
use discover::DiscoverState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(DesktopState::default())
        .manage(DiscoverState::default())
        .invoke_handler(tauri::generate_handler![
            assets::list_assets,
            assets::list_symbols,
            assets::list_search_range_profiles,
            assets::save_search_range_profile,
            assets::delete_search_range_profile,
            assets::list_discover_profiles,
            assets::save_discover_profile,
            assets::delete_discover_profile,
            assets::upsert_asset,
            assets::delete_asset,
            databank::load_databank,
            databank::get_elite,
            databank::get_elite_partition_equity,
            databank::run_elite_robustness,
            databank::export_elite_strategy,
            databank::export_elite_strategies,
            databank::export_elite_eas,
            databank::promote_elite_to_vault,
            databank::run_fidelity_demo,
            data_lab::inspect_data,
            data_lab::import_market_folder,
            discover::start_discover,
            discover::run_condition_bakeoff,
            discover::get_discover_job,
            discover::pause_discover,
            discover::resume_discover,
            discover::stop_discover,
            challenge::run_challenge_workflow,
            promotion_ledger::run_sealed_final,
            promotion_ledger::start_incubation,
            promotion_ledger::record_incubation,
            promotion_ledger::finalize_incubation,
            evidence::assemble_evidence,
            parity_lab::run_m1_judge,
            parity_lab::export_mql5,
            parity_lab::compare_external_parity,
            parity_lab::compare_indicator_parity,
            portfolio::build_portfolio,
            vault::inspect_vault,
            vault::certify_to_vault,
            deploy::build_deployment_pack
        ])
        .run(tauri::generate_context!())
        .expect("failed to run QuantForge desktop");
}

#[cfg(test)]
mod capability_tests {
    use serde_json::Value;

    #[test]
    fn main_window_can_open_and_save_through_the_native_dialog() {
        let capability: Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        let permissions = capability["permissions"].as_array().unwrap();

        for required in ["dialog:allow-open", "dialog:allow-save"] {
            assert!(
                permissions.iter().any(|permission| permission == required),
                "desktop capability is missing {required}"
            );
        }
    }
}
