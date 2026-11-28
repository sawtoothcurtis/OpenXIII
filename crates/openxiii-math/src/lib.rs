//! Transcendental functions used by the game's simulation.
//!
//! By default every function is the platform's own (`f32::sin` and friends: the C runtime of
//! the machine), exactly as before this module existed. In [`Mode::Portable`] (a network co-op session played in lockstep, where every machine must
//! compute the same bits), the functions come from `libm`, a pure Rust implementation that gives
//! the same result on every target and operating system.
//!
//! The functions are methods of [`SimMath`], implemented for `f32` and `f64`: `x.dsin()` is
//! `x.sin()` outside the portable mode. `sqrt` is exact everywhere (IEEE 754) and stays as it is.

use std::cell::Cell;

/// The math implementation for one game or a worker executing part of its work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Platform,
    Portable,
}

impl Default for Mode {
    fn default() -> Self {
        { Mode::Platform }
    }
}

thread_local! {
    // Only an operation's dynamic scope. The lasting choice belongs to the game.
    static ACTIVE: Cell<Option<Mode>> = const { Cell::new(None) };
}

impl Mode {
    #[inline]
    pub fn current() -> Self {
        ACTIVE.get().unwrap_or_default()
    }

    /// Use this game's math for a synchronous operation, including nested calls.
    #[inline]
    #[must_use]
    pub fn enter(self) -> Scope {
        let previous = ACTIVE.get();
        let changed = previous.unwrap_or_default() != self;
        if changed { ACTIVE.set(Some(self)); }
        Scope { previous, changed, thread: std::marker::PhantomData }
    }
}

/// Restores the caller's math on normal return or unwinding, on the same thread.
pub struct Scope {
    previous: Option<Mode>,
    changed: bool,
    thread: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl Drop for Scope {
    #[inline]
    fn drop(&mut self) {
        if self.changed { ACTIVE.set(self.previous); }
    }
}

/// Capture the caller's mode when submitting work, then install it on the worker.
pub fn inherit<F, R>(work: F) -> impl FnOnce() -> R
where
    F: FnOnce() -> R,
{
    let mode = Mode::current();
    move || {
        let _scope = mode.enter();
        work()
    }
}

#[inline]
pub fn portable() -> bool {
    Mode::current() == Mode::Portable
}

/// The simulation's transcendental functions (see the module).
pub trait SimMath: Sized + Copy {
    fn dsin(self) -> Self;
    fn dcos(self) -> Self;
    fn dtan(self) -> Self;
    fn dasin(self) -> Self;
    fn dacos(self) -> Self;
    fn datan(self) -> Self;
    fn datan2(self, other: Self) -> Self;
    fn dsin_cos(self) -> (Self, Self);
    fn dhypot(self, other: Self) -> Self;
    fn dpowf(self, n: Self) -> Self;
    fn dexp(self) -> Self;
    fn dexp2(self) -> Self;
    fn dln(self) -> Self;
    fn dlog(self, base: Self) -> Self;
    fn dlog2(self) -> Self;
    fn dlog10(self) -> Self;
}

macro_rules! sim_math {
    ($t:ty, $sin:path, $cos:path, $tan:path, $asin:path, $acos:path, $atan:path, $atan2:path, $hypot:path, $pow:path, $exp:path, $exp2:path, $ln:path, $log2:path, $log10:path) => {
        impl SimMath for $t {
            #[inline]
            fn dsin(self) -> Self { if portable() { $sin(self) } else { self.sin() } }
            #[inline]
            fn dcos(self) -> Self { if portable() { $cos(self) } else { self.cos() } }
            #[inline]
            fn dtan(self) -> Self { if portable() { $tan(self) } else { self.tan() } }
            #[inline]
            fn dasin(self) -> Self { if portable() { $asin(self) } else { self.asin() } }
            #[inline]
            fn dacos(self) -> Self { if portable() { $acos(self) } else { self.acos() } }
            #[inline]
            fn datan(self) -> Self { if portable() { $atan(self) } else { self.atan() } }
            #[inline]
            fn datan2(self, other: Self) -> Self { if portable() { $atan2(self, other) } else { self.atan2(other) } }
            #[inline]
            fn dsin_cos(self) -> (Self, Self) { if portable() { ($sin(self), $cos(self)) } else { self.sin_cos() } }
            #[inline]
            fn dhypot(self, other: Self) -> Self { if portable() { $hypot(self, other) } else { self.hypot(other) } }
            #[inline]
            fn dpowf(self, n: Self) -> Self { if portable() { $pow(self, n) } else { self.powf(n) } }
            #[inline]
            fn dexp(self) -> Self { if portable() { $exp(self) } else { self.exp() } }
            #[inline]
            fn dexp2(self) -> Self { if portable() { $exp2(self) } else { self.exp2() } }
            #[inline]
            fn dln(self) -> Self { if portable() { $ln(self) } else { self.ln() } }
            // std: self.ln() / base.ln().
            #[inline]
            fn dlog(self, base: Self) -> Self { if portable() { $ln(self) / $ln(base) } else { self.log(base) } }
            #[inline]
            fn dlog2(self) -> Self { if portable() { $log2(self) } else { self.log2() } }
            #[inline]
            fn dlog10(self) -> Self { if portable() { $log10(self) } else { self.log10() } }
        }
    };
}

sim_math!(f32, libm::sinf, libm::cosf, libm::tanf, libm::asinf, libm::acosf, libm::atanf, libm::atan2f, libm::hypotf, libm::powf, libm::expf, libm::exp2f, libm::logf, libm::log2f, libm::log10f);
sim_math!(f64, libm::sin, libm::cos, libm::tan, libm::asin, libm::acos, libm::atan, libm::atan2, libm::hypot, libm::pow, libm::exp, libm::exp2, libm::log, libm::log2, libm::log10);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_modes_restore_the_caller_and_follow_its_worker() {
        let original = Mode::current();
        let outer = Mode::Platform.enter();
        let worker = {
            let _inner = Mode::Portable.enter();
            assert!(portable());
            inherit(|| {
                assert!(portable());
                let _nested = Mode::Platform.enter();
                assert!(!portable());
            })
        };
        assert!(!portable());
        std::thread::spawn(worker).join().unwrap();
        assert!(!portable());
        drop(outer);
        assert_eq!(Mode::current(), original);
    }

    /// How often the platform's functions and the portable ones differ at all (printed: run with
    /// --nocapture), over a spread of f32 inputs. Not an assertion: the platform decides.
    #[test]
    fn platform_and_portable_differences() {
        let mut n = 0u32;
        let mut diff = [0u32; 8];
        for i in 0..200_000u32 {
            let x = (i as f32 - 100_000.0) * 0.000_731 + (i % 7) as f32 * 1.0e-4;
            n += 1;
            let pairs = [
                (x.sin(), libm::sinf(x)),
                (x.cos(), libm::cosf(x)),
                (x.tan(), libm::tanf(x)),
                (x.atan(), libm::atanf(x)),
                (x.atan2(0.37), libm::atan2f(x, 0.37)),
                (x.abs().ln(), libm::logf(x.abs())),
                ((x * 0.01).exp(), libm::expf(x * 0.01)),
                (x.abs().powf(1.7), libm::powf(x.abs(), 1.7)),
            ];
            for (k, (a, b)) in pairs.iter().enumerate() {
                if a.to_bits() != b.to_bits() && !(a.is_nan() && b.is_nan()) {
                    diff[k] += 1;
                }
            }
        }
        println!("f32 inputs {n}: different results sin {} cos {} tan {} atan {} atan2 {} ln {} exp {} powf {}", diff[0], diff[1], diff[2], diff[3], diff[4], diff[5], diff[6], diff[7]);
        // f64 (the scripts' natives compute in double then round to single).
        let mut d64 = [0u32; 3];
        for i in 0..200_000u32 {
            let x = (i as f64 - 100_000.0) * 0.000_731;
            if x.sin().to_bits() != libm::sin(x).to_bits() { d64[0] += 1; }
            if x.cos().to_bits() != libm::cos(x).to_bits() { d64[1] += 1; }
            if x.atan().to_bits() != libm::atan(x).to_bits() { d64[2] += 1; }
        }
        println!("f64 inputs 200000: different results sin {} cos {} atan {}", d64[0], d64[1], d64[2]);
        println!("portable mode from the environment: {}", portable());
    }

    #[test]
    fn both_modes_agree_closely() {
        // Not bit-identical in general (that is the point), but within a few ulps.
        for i in -2000..2000 {
            let x = i as f32 * 0.00731;
            let close = |a: f32, b: f32| (a - b).abs() <= 4.0 * f32::EPSILON * a.abs().max(1.0);
            assert!(close(x.sin(), libm::sinf(x)));
            assert!(close(x.cos(), libm::cosf(x)));
            assert!(close(x.atan(), libm::atanf(x)));
            assert!(close(x.atan2(0.37), libm::atan2f(x, 0.37)));
        }
    }
}
