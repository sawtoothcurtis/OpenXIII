//! Fixed settings of the HX audio engine: the values the game always uses.

pub(crate) fn static_at_turns() -> bool {
    true
}

pub(crate) fn static_delay() -> f64 {
    crate::stream::ds8_latency()
}

pub(crate) fn mode_advance() -> Option<&'static str> {
    Some("1")
}

pub fn ds8_latency() -> f64 {
    crate::stream::DS8_LATENCY_DEFAULT
}

pub(crate) fn trace_calls() -> bool {
    false
}

pub(crate) fn use_audio_thread() -> bool {
    true
}

pub(crate) fn accept_recorded_ds8_period() -> bool {
    true
}

pub(crate) fn accept_recorded_clock_ratio() -> bool {
    true
}
