# wgpu-hal 30.0.1, as patched for OpenXIII

This is wgpu-hal 30.0.1 from crates.io, copied as is except for the changes below. OpenXIII uses it
through `[patch.crates-io]` in the workspace `Cargo.toml`; removing that entry goes back to the
crates.io version (without the Direct3D 11 backend). Search for "OpenXIII" in `src` to find the changes.

- The examples were removed (`examples` folder and the `[[example]]` sections of `Cargo.toml`).
- `src/gles/wgl.rs`: the OpenGL (WGL) context stays current between two locks
  (`AdapterContextLock::drop` no longer releases it, `WglContext::make_current` does nothing when it is
  already current with the same DC). Without this, every buffer creation, write or upload made two
  `wglMakeCurrent` calls: on graphics cards without DirectX 12 or Vulkan (OpenGL only), that was a
  quarter of a frame's time. OpenXIII draws from a single thread.
- Vertex formats: identical vertex buffer layout descriptions are cached per slot on the
  `VERTEX_BUFFER_LAYOUT` path, reset for each encoding.
- Uploads: in `src/gles/device.rs` `create_buffer`, wgpu-core's staging buffers (`MAP_WRITE | COPY_SRC`,
  transient) stay in CPU memory (the `EMULATE_BUFFER_MAP` path): the copy becomes `glBufferSubData` /
  `glTexSubImage` from memory, instead of creating, mapping and destroying an OpenGL buffer on every
  `Queue::write_buffer`. `COPY_DST` buffers get `DYNAMIC_STORAGE_BIT` (required by `glBufferSubData` on
  immutable storage) without adding it to the mapping flags. In `src/gles/queue.rs`
  `CopyBufferToTexture`, `PIXEL_UNPACK_BUFFER` is unbound before an upload from memory.
- State cache between passes (`src/gles/cache_state.rs`): the
  state set by the queue (program, tests, index buffer, uniform ranges, textures and samplers per unit,
  faces, depth, color, blending) is remembered and a call that would set the same value again is not
  issued. It is forgotten on each context lock (`wgl.rs`, `egl.rs`, `web.rs`) and after commands that
  bind other objects on the way (copies, barriers, clears by shader).
- `src/dx11`: a Direct3D 11 backend written for OpenXIII, used instead of DirectX 12 when the `dx11`
  feature is enabled (for graphics cards without DirectX 12).