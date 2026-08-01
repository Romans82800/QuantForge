//! Genetic islands with ring migration for Mass Builder scale.
//!
//! When `complex_m1_island_count > 0`, islands split into two bands:
//! - lower ids: Selected-TF simple (market-only)
//! - higher ids: complex M1 (pending / BE / trail / partials)
//! Migration stays inside each band so profiles do not cross-contaminate.

use crate::model::{Databank, Elite};
use rand::Rng;
use rand_chacha::ChaCha8Rng;

/// Pick a tournament winner constrained to one breeding island.
pub(crate) fn tournament_on_island(
    bank: &Databank,
    rng: &mut ChaCha8Rng,
    island_id: u16,
) -> Option<usize> {
    let island_indices: Vec<usize> = bank
        .accepted_pool
        .iter()
        .enumerate()
        .filter(|(_, elite)| elite.island_id == island_id)
        .map(|(index, _)| index)
        .collect();
    if island_indices.is_empty() {
        return None;
    }
    let size = bank.config.tournament_size.max(1).min(island_indices.len());
    let mut best = island_indices[rng.gen_range(0..island_indices.len())];
    for _ in 1..size {
        let challenger = island_indices[rng.gen_range(0..island_indices.len())];
        if tournament_score(&bank.accepted_pool[challenger])
            > tournament_score(&bank.accepted_pool[best])
        {
            best = challenger;
        }
    }
    Some(best)
}

fn tournament_score(elite: &Elite) -> f64 {
    elite.evidence.total + elite.novelty * 0.01 - elite.complexity as f64 * 0.001
}

fn band_destination(bank: &Databank, island: usize) -> u16 {
    let total = bank.config.effective_island_count();
    let complex = bank.config.effective_complex_m1_islands();
    if complex == 0 || complex >= total {
        return ((island + 1) % total) as u16;
    }
    let first_complex = total.saturating_sub(complex);
    if island < first_complex {
        // Simple band ring.
        let size = first_complex.max(1);
        ((island + 1) % size) as u16
    } else {
        // Complex band ring.
        let offset = island - first_complex;
        let size = complex.max(1);
        (first_complex + ((offset + 1) % size)) as u16
    }
}

/// Move top elites within their profile band (simple↔simple, complex↔complex).
pub(crate) fn migrate_islands(bank: &mut Databank) -> u64 {
    let island_count = bank.config.effective_island_count();
    if island_count <= 1 || bank.config.migration_elites == 0 {
        return 0;
    }
    let k = bank.config.migration_elites;
    let mut moves = Vec::new();
    for island in 0..island_count {
        let island_id = island as u16;
        let mut members: Vec<usize> = bank
            .accepted_pool
            .iter()
            .enumerate()
            .filter(|(_, elite)| elite.island_id == island_id)
            .map(|(index, _)| index)
            .collect();
        members.sort_by(|&left, &right| {
            tournament_score(&bank.accepted_pool[right])
                .partial_cmp(&tournament_score(&bank.accepted_pool[left]))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let destination = band_destination(bank, island);
        for &index in members.iter().take(k) {
            moves.push((index, destination));
        }
    }
    for (index, destination) in &moves {
        bank.accepted_pool[*index].island_id = *destination;
    }
    let migrated = moves.len() as u64;
    bank.telemetry.island_migrations += migrated;
    migrated
}
