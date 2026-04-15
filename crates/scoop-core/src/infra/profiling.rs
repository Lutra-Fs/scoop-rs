use std::time::{Duration, Instant};

fn profiling_enabled() -> bool {
    std::env::var_os("SCOOP_RS_PROFILE").is_some()
}

pub struct ScopeTimer {
    enabled: bool,
    label: String,
    start: Instant,
}

impl ScopeTimer {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            enabled: profiling_enabled(),
            label: label.into(),
            start: Instant::now(),
        }
    }
}

impl Drop for ScopeTimer {
    fn drop(&mut self) {
        if self.enabled {
            emit_duration(&self.label, self.start.elapsed());
        }
    }
}

pub fn scope(label: impl Into<String>) -> ScopeTimer {
    ScopeTimer::new(label)
}

pub fn emit_duration(label: &str, duration: Duration) {
    if profiling_enabled() {
        eprintln!(
            "[profile] {label}: {:.3} ms",
            duration.as_secs_f64() * 1000.0
        );
    }
}
