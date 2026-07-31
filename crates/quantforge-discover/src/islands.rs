//! Genetic islands with ring migration for Mass Builder scale.

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

/// Move top elites from each island onto the next island (ring topology).
/// Fingerprints stay unique in the breeding bag; only `island_id` changes.
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
        let destination = ((island + 1) % island_count) as u16;
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
