//! Opt-in timing events for performance investigation.

use std::{sync::OnceLock, time::Instant};

const TRACE_TIMING: &str = "MNEMOSYNE_TRACE_TIMING";

pub struct Scope {
    phase: &'static str,
    started: Option<Instant>,
}

impl Scope {
    pub fn new(phase: &'static str) -> Self {
        Self {
            phase,
            started: enabled().then(Instant::now),
        }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        if let Some(started) = self.started {
            emit(
                self.phase,
                "measured",
                Some(started.elapsed().as_secs_f64() * 1_000.0),
            );
        }
    }
}

pub fn not_run(phase: &'static str) {
    if enabled() {
        emit(phase, "not_run", None);
    }
}

fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var(TRACE_TIMING).ok().as_deref() == Some("1"))
}

fn emit(phase: &str, status: &str, duration_ms: Option<f64>) {
    eprintln!(
        "{}",
        serde_json::json!({
            "event": "mnemosyne_timing",
            "phase": phase,
            "status": status,
            "inclusive": true,
            "duration_ms": duration_ms,
        })
    );
}
