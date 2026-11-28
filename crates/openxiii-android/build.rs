//! Build script: the lockstep build identity (OXIII_BUILD_ID), shared with openxiii-viewer's build.rs.
#[path = "../openxiii-viewer/build_id.rs"]
mod build_id;

fn main() {
    build_id::configure();
    println!("cargo:rerun-if-changed=build.rs");
}
