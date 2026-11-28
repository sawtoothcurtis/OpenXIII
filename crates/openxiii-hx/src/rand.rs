//! `rand()` from the Visual C++ C library (MSVCR70.DLL), used everywhere in HXAudio.
//!
//! The seed is per thread in this library: the game thread and the sound update thread
//! (`UHXAUDIOSubsystem::ParallelUpdateThread`, seed 1 by default) each have
//! their own sequence. On the game thread, this sequence is also that of Core's appRand / appFrand
//! (same msvcr70): the VM passes its seed to `Engine::rand_main` before each call into HXAudio and
//! takes it back afterwards.
//!
//! The CUUID of a new object (`srand(rdtsc >> 32)` the first time, then one
//! `rand()`) is drawn for each new HX actor (actor resource created for an
//! Unreal actor): e.g. on Hual01a, 1,168 draws at load time and 6 in frames 2 and 11.
//! At the first call (Engine::cuuid_rand), the game seeds from the clock like the original
//! (`Engine::srand_clock`); tests and tools leave it off and the sequence continues.

#[derive(Debug, Clone)]
pub struct MsvcRand {
    pub seed: u32,
    /// Number of draws made on this sequence (draw count per VM caller).
    pub draws: u64,
}

impl MsvcRand {
    pub fn new(seed: u32) -> Self {
        MsvcRand { seed, draws: 0 }
    }

    /// `rand()`: 0..=0x7FFF.
    pub fn next(&mut self) -> i32 {
        self.draws += 1;
        self.seed = self.seed.wrapping_mul(214013).wrapping_add(2531011);
        ((self.seed >> 16) & 0x7fff) as i32
    }
}

impl Default for MsvcRand {
    fn default() -> Self {
        MsvcRand::new(1)
    }
}

/// The value gives srand on the first CUUID (`rdtsc`, then
/// srand(edx), the high dword of the time-stamp counter). Elsewhere than x86, the high dword of a
/// nanosecond clock, its nearest equivalent.
#[allow(unused_unsafe, unreachable_code)]
pub fn clock_srand_seed() -> u32 {
    #[cfg(target_arch = "x86_64")]
    return (unsafe { core::arch::x86_64::_rdtsc() } >> 32) as u32;
    #[cfg(target_arch = "x86")]
    return (unsafe { core::arch::x86::_rdtsc() } >> 32) as u32;
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    (nanos >> 32) as u32
}
