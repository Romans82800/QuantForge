//! Deterministic genetic islands with ring migration.

use crate::model::{Databank, Elite};
use rand::Rng;
use rand_chacha::ChaCha8Rng;

pub(crate) fn tournament_on_island(
    bank: &Databank,
    rng: &mut ChaCha8Rng,
    island_id: u16,
) -> Option<usize> {
    let members: Vec<usize> = bank
        .accepted_pool
        .iter()
        .enumerate()
        .filter(|(_, elite)| elite.island_id == island_id)
        .map(|(index, _)| index)
        .collect();
    if members.is_empty() {
        return None;
    }
    let rounds = bank.config.tournament_size.max(1).min(members.len());
    let mut best = members[rng.gen_range(0..members.len())];
    for _ in 1..rounds {
        let challenger = members[rng.gen_range(0..members.len())];
        if score(&bank.accepted_pool[challenger]) > score(&bank.accepted_pool[best]) {
            best = challenger;
        }
    }
    Some(best)
}

pub(crate) fn tournament_in_elites(
    entries: &[Elite],
    config: &crate::model::DiscoverConfig,
    rng: &mut ChaCha8Rng,
    island_id: u16,
) -> Option<usize> {
    let members: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, elite)| elite.island_id == island_id)
        .map(|(index, _)| index)
        .collect();
    if members.is_empty() {
        return None;
    }
    let rounds = config.tournament_size.max(1).min(members.len());
    let mut best = members[rng.gen_range(0..members.len())];
    for _ in 1..rounds {
        let challenger = members[rng.gen_range(0..members.len())];
        if score(&entries[challenger]) > score(&entries[best]) {
            best = challenger;
        }
    }
    Some(best)
}

pub(crate) fn migrate_islands(bank: &mut Databank) {
    let count = bank.config.effective_island_count();
    if count < 2 || bank.config.migration_elites == 0 {
        return;
    }
    // Only complete Development-battery survivors may migrate. OOS1 never
    // influences this pool, and raw pot candidates cannot leak across islands.
    let mut moves = Vec::new();
    for source in 0..count {
        let mut members: Vec<usize> = bank
            .specialist_pool
            .iter()
            .enumerate()
            .filter(|(_, elite)| elite.island_id as usize == source)
            .map(|(index, _)| index)
            .collect();
        members.sort_by(|a, b| {
            score(&bank.specialist_pool[*b]).total_cmp(&score(&bank.specialist_pool[*a]))
        });
        for index in members.into_iter().take(bank.config.migration_elites) {
            moves.push((index, ((source + 1) % count) as u16));
        }
    }
    for (index, destination) in moves {
        let fingerprint = bank.specialist_pool[index].structural_fingerprint.clone();
        bank.specialist_pool[index].island_id = destination;
        if let Some(parent) = bank
            .accepted_pool
            .iter_mut()
            .find(|elite| elite.structural_fingerprint == fingerprint)
        {
            parent.island_id = destination;
        }
        bank.telemetry.island_migrations += 1;
    }
}

fn score(elite: &Elite) -> f64 {
    elite.evidence.total + elite.novelty * 2.0 - elite.complexity as f64 * 0.05
}
