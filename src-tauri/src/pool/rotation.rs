use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::settings::Rotation;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bridge {
    pub config_id: i64,
    pub port: u16,
}

struct Entry {
    bridge: Bridge,
    failures: u32,
    excluded: bool,
}

/// Thread-safe rotation state. Connections are handed to the next live bridge;
/// three setup failures in a row park a bridge until it recovers or the pool
/// is restarted. Database status is never changed here.
pub struct RotationState {
    entries: RwLock<Vec<Entry>>,
    strategy: RwLock<Rotation>,
    cursor: AtomicUsize,
}

impl RotationState {
    pub fn new(strategy: Rotation, bridges: Vec<Bridge>) -> Self {
        Self {
            entries: RwLock::new(
                bridges
                    .into_iter()
                    .map(|bridge| Entry {
                        bridge,
                        failures: 0,
                        excluded: false,
                    })
                    .collect(),
            ),
            strategy: RwLock::new(strategy),
            cursor: AtomicUsize::new(0),
        }
    }

    pub fn set_strategy(&self, strategy: Rotation) {
        if let Ok(mut current) = self.strategy.write() {
            *current = strategy;
        }
    }

    pub fn strategy(&self) -> Rotation {
        self.strategy
            .read()
            .map(|s| *s)
            .unwrap_or(Rotation::RoundRobin)
    }

    pub fn len(&self) -> usize {
        self.entries
            .read()
            .map(|entries| entries.iter().filter(|e| !e.excluded).count())
            .unwrap_or(0)
    }

    /// Live bridges in the order this connection should try them.
    pub fn candidates(&self) -> Vec<Bridge> {
        let Ok(mut entries) = self.entries.write() else {
            return Vec::new();
        };
        let mut available: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.excluded)
            .map(|(index, _)| index)
            .collect();
        if available.is_empty() {
            // Everything was parked: give the pool another chance rather than
            // letting it fail forever.
            for entry in entries.iter_mut() {
                entry.excluded = false;
                entry.failures = 0;
            }
            available = (0..entries.len()).collect();
        }
        if available.is_empty() {
            return Vec::new();
        }
        let strategy = self.strategy.read().map(|s| *s).unwrap_or(Rotation::RoundRobin);
        let start = match strategy {
            Rotation::RoundRobin => self.cursor.fetch_add(1, Ordering::Relaxed),
            Rotation::Random => SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.subsec_nanos() as usize)
                .unwrap_or(0),
        } % available.len();
        (0..available.len())
            .map(|offset| entries[available[(start + offset) % available.len()]].bridge.clone())
            .collect()
    }

    pub fn note_success(&self, config_id: i64) {
        if let Ok(mut entries) = self.entries.write() {
            if let Some(entry) = entries.iter_mut().find(|e| e.bridge.config_id == config_id) {
                entry.failures = 0;
                entry.excluded = false;
            }
        }
    }

    pub fn note_failure(&self, config_id: i64) {
        if let Ok(mut entries) = self.entries.write() {
            if let Some(entry) = entries.iter_mut().find(|e| e.bridge.config_id == config_id) {
                entry.failures += 1;
                if entry.failures >= 3 {
                    entry.excluded = true;
                }
            }
        }
    }

    /// A configuration tested dead is removed from the live pool immediately.
    pub fn remove(&self, config_id: i64) {
        if let Ok(mut entries) = self.entries.write() {
            entries.retain(|e| e.bridge.config_id != config_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> RotationState {
        RotationState::new(
            Rotation::RoundRobin,
            vec![
                Bridge {
                    config_id: 1,
                    port: 1001,
                },
                Bridge {
                    config_id: 2,
                    port: 1002,
                },
                Bridge {
                    config_id: 3,
                    port: 1003,
                },
            ],
        )
    }

    #[test]
    fn round_robin_rotates_the_starting_bridge() {
        let rotation = state();
        assert_eq!(rotation.candidates()[0].config_id, 1);
        assert_eq!(rotation.candidates()[0].config_id, 2);
        assert_eq!(rotation.candidates()[0].config_id, 3);
        assert_eq!(rotation.candidates()[0].config_id, 1);
        for candidate_set in [rotation.candidates(), rotation.candidates()] {
            let mut ids: Vec<i64> = candidate_set.iter().map(|b| b.config_id).collect();
            ids.sort_unstable();
            assert_eq!(ids, vec![1, 2, 3]);
        }
    }

    #[test]
    fn three_failures_park_a_bridge_and_success_recovers_it() {
        let rotation = state();
        for _ in 0..3 {
            rotation.note_failure(2);
        }
        let ids: Vec<i64> = rotation.candidates().iter().map(|b| b.config_id).collect();
        assert!(!ids.contains(&2));
        rotation.note_success(2);
        let ids: Vec<i64> = rotation.candidates().iter().map(|b| b.config_id).collect();
        assert!(ids.contains(&2));
    }

    #[test]
    fn dead_configs_leave_the_pool() {
        let rotation = state();
        rotation.remove(1);
        assert_eq!(rotation.len(), 2);
        let ids: Vec<i64> = rotation.candidates().iter().map(|b| b.config_id).collect();
        assert!(!ids.contains(&1));
    }

    #[test]
    fn all_parked_bridges_are_retried() {
        let rotation = state();
        for id in 1..=3 {
            for _ in 0..3 {
                rotation.note_failure(id);
            }
        }
        assert_eq!(rotation.candidates().len(), 3);
    }
}
