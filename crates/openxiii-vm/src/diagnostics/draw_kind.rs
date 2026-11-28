//! Kind of a rand() draw on the game thread's sequence (draw diagnostics): S = Core's appFrand
//! (the script's FRand, VRand…), R = the script's appRand (Rand), N = draw from a native module
//! (Engine, XIDPawn, D3DDrv…: particles, native AI), H = HXAudio.

pub const S: u8 = b'S';
pub const R: u8 = b'R';
pub const N: u8 = b'N';
pub const H: u8 = b'H';
