# wgpu-core 30.0.1, as patched for OpenXIII

This is wgpu-core 30.0.1 from crates.io (copied on 2026-11-04), modified for OpenXIII. The workspace
`Cargo.toml` uses it through `[patch.crates-io] wgpu-core = { path = "vendor/wgpu-core" }`. Search for
"OpenXIII" in `src` to find the changes.

- `command/pass.rs`: within one pass, a bind group already merged into the pass's usages is not merged
  again, and its initialization actions (buffers, textures) are recorded only once. Same result (the
  later times only added duplicates), less work per draw.
