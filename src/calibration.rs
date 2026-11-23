use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_CONCURRENCY: u16 = 16;
pub const MIN_DEBOUNCE_MS: u64 = 100;
pub const MAX_DEBOUNCE_MS: u64 = 2500;
const FRESH_FOR_SECS: u64 = 7 * 24 * 60 * 60;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Limits {
    pub concurrent: u16,
    pub requests_per_second: f64,
    pub debounce_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            concurrent: 1,
            requests_per_second: 2.0,
            debounce_ms: 650,
        }
    }
}

impl Limits {
    fn bounded(self) -> Self {
        Self {
            concurrent: self.concurrent.clamp(1, MAX_CONCURRENCY),
            requests_per_second: self.requests_per_second.max(0.01),
            debounce_ms: self.debounce_ms.clamp(MIN_DEBOUNCE_MS, MAX_DEBOUNCE_MS),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Measurement {
    pub limits: Limits,
    pub measured_at: u64,
    pub sequential_p95_ms: u64,
    pub p95_ms: u64,
    pub throughput: f64,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Overrides {
    pub concurrent: Option<u16>,
    pub requests_per_second: Option<f64>,
    pub debounce_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct Profile {
    pub measurement: Option<Measurement>,
    pub overrides: Overrides,
}

impl Profile {
    pub fn limits(&self) -> Limits {
        let base = self.measurement.map_or_else(Limits::default, |m| m.limits);
        Limits {
            concurrent: self.overrides.concurrent.unwrap_or(base.concurrent),
            requests_per_second: self
                .overrides
                .requests_per_second
                .unwrap_or(base.requests_per_second),
            debounce_ms: self.overrides.debounce_ms.unwrap_or(base.debounce_ms),
        }
        .bounded()
    }

    pub fn stale(&self, now: u64) -> bool {
        self.measurement
            .is_none_or(|m| m.measured_at > now || now - m.measured_at >= FRESH_FOR_SECS)
    }
}

pub fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(all(windows, target_arch = "x86_64"))]
mod run;
#[cfg(all(windows, target_arch = "x86_64"))]
pub use run::{BUDGET, measure};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expires_measurements_and_applies_only_the_overridden_limits() {
        let profile = Profile {
            measurement: Some(Measurement {
                limits: Limits {
                    concurrent: 4,
                    requests_per_second: 8.0,
                    debounce_ms: 200,
                },
                measured_at: 100,
                sequential_p95_ms: 150,
                p95_ms: 310,
                throughput: 10.0,
            }),
            overrides: Overrides {
                concurrent: Some(2),
                ..Default::default()
            },
        };
        assert_eq!(profile.limits().concurrent, 2);
        assert_eq!(profile.limits().requests_per_second, 8.0);
        assert_eq!(profile.limits().debounce_ms, 200);
        assert!(!profile.stale(100 + FRESH_FOR_SECS - 1));
        assert!(profile.stale(100 + FRESH_FOR_SECS));
        assert!(Profile::default().stale(100));
    }
}
