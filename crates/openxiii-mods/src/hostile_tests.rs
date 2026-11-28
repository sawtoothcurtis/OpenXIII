//! Broken or hostile mod files: each one is refused with an error (or read safely), never a
//! panic, a hang, a stack overflow or an allocation sized by the file's own counts.
use crate::own_animated::load_animated_gltf;
use crate::own_models::load_gltf;
use std::path::{Path, PathBuf};

/// The one-triangle model of own_models' tests (44-byte .bin, 3 positions and 3 u16 indices),
/// with `edit` applied to its glTF text.
fn triangle_with(dir: &Path, edit: impl Fn(String) -> String) -> PathBuf {
    let path = crate::own_models::tests::write_triangle(dir);
    edit_file(&path, edit);
    path
}

fn edit_file(path: &Path, edit: impl Fn(String) -> String) {
    let json = std::fs::read_to_string(path).unwrap();
    let edited = edit(json.clone());
    assert_ne!(edited, json, "the edit changed nothing");
    std::fs::write(path, edited).unwrap();
}

/// Short form of the result (a refused model's error, or what was read).
fn refused<T>(r: Result<T, String>, want: &str) {
    match r {
        Ok(_) => panic!("accepted, expected an error saying {want:?}"),
        Err(e) => assert!(e.contains(want), "error {e:?} does not say {want:?}"),
    }
}

const TRIANGLE_NODES: &str = r#""nodes": [{"mesh": 0, "translation": [0, 0, -2]}]"#;

#[test]
fn the_unchanged_triangle_loads() {
    let tmp = tempfile::tempdir().unwrap();
    let path = crate::own_models::tests::write_triangle(tmp.path());
    assert_eq!(load_gltf(&path, 50.0).unwrap().len(), 1);
}

#[test]
fn self_referencing_node_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let path = triangle_with(tmp.path(), |j| j.replace(TRIANGLE_NODES, r#""nodes": [{"mesh": 0, "children": [0]}]"#));
    refused(load_gltf(&path, 50.0), "own ancestor");
    refused(load_animated_gltf(&path, 50.0), "own ancestor");
}

#[test]
fn node_loop_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let path = triangle_with(tmp.path(), |j| j.replace(TRIANGLE_NODES, r#""nodes": [{"mesh": 0, "children": [1]}, {"children": [0]}]"#));
    refused(load_gltf(&path, 50.0), "own ancestor");
    refused(load_animated_gltf(&path, 50.0), "own ancestor");
}

#[test]
fn deep_node_chain_does_not_overflow_the_stack() {
    // 200 000 nodes, each the child of the previous one.
    let tmp = tempfile::tempdir().unwrap();
    let n = 200_000;
    let nodes: Vec<String> = (0..n).map(|i| if i + 1 < n { format!("{{\"children\": [{}]}}", i + 1) } else { "{\"mesh\": 0}".to_string() }).collect();
    let path = triangle_with(tmp.path(), |j| j.replace(TRIANGLE_NODES, &format!("\"nodes\": [{}]", nodes.join(","))));
    assert_eq!(load_gltf(&path, 50.0).unwrap().len(), 1);
    refused(load_animated_gltf(&path, 50.0), "no skinned mesh");
}

#[test]
fn deep_skeleton_does_not_overflow_the_stack() {
    // The skinned arm of own_animated's tests, its joint "Hips" under a chain of 200 000 nodes
    // (the matrices of the nodes used to be computed recursively).
    let tmp = tempfile::tempdir().unwrap();
    let path = crate::own_animated::tests::write_arm(tmp.path());
    let n = 200_000;
    let chain: String = (0..n).map(|i| format!(",{{\"children\": [{}]}}", if i + 1 < n { 3 + i + 1 } else { 0 })).collect();
    edit_file(&path, |j| {
        j.replace(r#""scenes": [{"nodes": [0, 2]}]"#, r#""scenes": [{"nodes": [3, 2]}]"#)
            .replace(r#"{"name": "Body", "mesh": 0, "skin": 0}"#, &format!("{{\"name\": \"Body\", \"mesh\": 0, \"skin\": 0}}{chain}"))
    });
    let model = load_animated_gltf(&path, 50.0).unwrap();
    assert_eq!(model.bones.len(), 3);
}

#[test]
fn paths_leaving_the_mod_folder_are_refused() {
    for uri in ["../tri.bin", "sub/../../tri.bin", "/tri.bin", "\\tri.bin", "C:/tri.bin", "C:tri.bin", "\\\\server\\share\\tri.bin"] {
        let tmp = tempfile::tempdir().unwrap();
        let inner = tmp.path().join("mod");
        std::fs::create_dir(&inner).unwrap();
        let json_uri = format!("\"{}\"", uri.replace('\\', "\\\\"));
        let path = triangle_with(&inner, |j| j.replace(r#""uri": "tri.bin""#, &format!("\"uri\": {json_uri}")));
        refused(load_gltf(&path, 50.0), "inside the mod folder");
        refused(load_animated_gltf(&path, 50.0), "inside the mod folder");
    }
    // A texture outside the folder is not used.
    let tmp = tempfile::tempdir().unwrap();
    let path = triangle_with(tmp.path(), |j| j.replace(r#""uri": "tri.png""#, r#""uri": "../tri.png""#));
    assert_eq!(load_gltf(&path, 50.0).unwrap()[0].texture, None);
}

#[test]
fn truncated_buffer_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let path = crate::own_models::tests::write_triangle(tmp.path());
    // The .bin cut to 10 bytes; the glTF still says 44.
    let bin = std::fs::read(tmp.path().join("tri.bin")).unwrap();
    std::fs::write(tmp.path().join("tri.bin"), &bin[..10]).unwrap();
    refused(load_gltf(&path, 50.0), "goes past the end of its buffer");
    refused(load_animated_gltf(&path, 50.0), "goes past the end of its buffer");
}

#[test]
fn view_past_the_buffer_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let path = triangle_with(tmp.path(), |j| j.replace(r#"{"buffer": 0, "byteOffset": 36, "byteLength": 6}"#, r#"{"buffer": 0, "byteOffset": 36, "byteLength": 600000}"#));
    refused(load_gltf(&path, 50.0), "goes past the end of its buffer");
}

#[test]
fn absurd_counts_are_refused() {
    // Accessors claiming a billion items over the 44-byte buffer: refused, not read.
    for (from, to) in [
        (r#""componentType": 5126, "count": 3,"#, r#""componentType": 5126, "count": 1000000000,"#),
        (r#""componentType": 5123, "count": 3,"#, r#""componentType": 5123, "count": 1000000000,"#),
        (r#""componentType": 5123, "count": 3,"#, r#""componentType": 5123, "count": 18446744073709551615,"#),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let path = triangle_with(tmp.path(), |j| j.replace(from, to));
        let e = load_gltf(&path, 50.0).unwrap_err();
        assert!(e.contains("more than its buffer view") || e.contains("tri.gltf"), "{e}");
        assert!(load_animated_gltf(&path, 50.0).is_err());
    }
    // No items, and an accessor without a buffer view (zeros with one sparse value) that claims a
    // billion items.
    for (from, to, want) in [
        (r#""componentType": 5126, "count": 3,"#, r#""componentType": 5126, "count": 0,"#, "no items"),
        (
            r#"{"bufferView": 0, "componentType": 5126, "count": 3,"#,
            r#"{"sparse": {"count": 1, "indices": {"bufferView": 1, "componentType": 5123}, "values": {"bufferView": 0}}, "componentType": 5126, "count": 1000000000,"#,
            "without a buffer view",
        ),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let path = triangle_with(tmp.path(), |j| j.replace(from, to));
        refused(load_gltf(&path, 50.0), want);
    }
}

#[test]
fn repeated_primitives_are_bounded() {
    // 400 000 primitives over the same 3 vertices: under the per-part limit, but the model
    // would hold 1 200 000 vertices in all.
    let tmp = tempfile::tempdir().unwrap();
    let prim = r#"{"attributes": {"POSITION": 0}, "indices": 1, "material": 0}"#;
    let many = vec![prim; 400_000].join(",");
    let path = triangle_with(tmp.path(), |j| j.replace(&format!("\"primitives\": [{prim}]"), &format!("\"primitives\": [{many}]")));
    refused(load_gltf(&path, 50.0), "vertices in all");
}

#[test]
fn repeated_nodes_are_bounded() {
    // The same mesh placed by 400 000 nodes.
    let tmp = tempfile::tempdir().unwrap();
    let nodes = vec![r#"{"mesh": 0}"#; 400_000].join(",");
    let roots = (0..400_000).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
    let path = triangle_with(tmp.path(), |j| j.replace(TRIANGLE_NODES, &format!("\"nodes\": [{nodes}]")).replace(r#""scenes": [{"nodes": [0]}]"#, &format!("\"scenes\": [{{\"nodes\": [{roots}]}}]")));
    refused(load_gltf(&path, 50.0), "vertices in all");
}

#[test]
fn skinned_vertices_are_bounded_before_reading() {
    // The arm's primitive repeated 20 000 times: 80 000 points, over the 65535 supported.
    let tmp = tempfile::tempdir().unwrap();
    let path = crate::own_animated::tests::write_arm(tmp.path());
    let prim = r#"{"attributes": {"POSITION": 0, "JOINTS_0": 1, "WEIGHTS_0": 2}, "indices": 3}"#;
    let many = vec![prim; 20_000].join(",");
    edit_file(&path, |j| j.replace(&format!("\"primitives\": [{prim}]"), &format!("\"primitives\": [{many}]")));
    refused(load_animated_gltf(&path, 50.0), "at most 65535 vertices");
}

#[test]
fn animations_are_bounded() {
    // The arm's one-second "wave" (30 samples of 3 bones) repeated 150 000 times: each clip
    // is short, all of them together are 13.5 million frames.
    let tmp = tempfile::tempdir().unwrap();
    let path = crate::own_animated::tests::write_arm(tmp.path());
    let wave = r#"{"name": "wave", "channels": [{"sampler": 0, "target": {"node": 1, "path": "rotation"}}], "samplers": [{"input": 5, "output": 6}]}"#;
    let many = vec![wave; 150_000].join(",");
    edit_file(&path, |j| j.replace(&format!("\"animations\": [{wave}]"), &format!("\"animations\": [{many}]")));
    refused(load_animated_gltf(&path, 50.0), "too long for this skeleton");
}

/// A PNG made of a signature, an IHDR chunk, an empty IDAT chunk and IEND (no pixels).
fn png_header(w: u32, h: u32) -> Vec<u8> {
    let chunk = |out: &mut Vec<u8>, body: &[u8]| {
        out.extend(((body.len() - 4) as u32).to_be_bytes());
        out.extend(body);
        out.extend(crc32(body).to_be_bytes());
    };
    let mut ihdr = b"IHDR".to_vec();
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]); // 8 bits, RGBA, deflate, no filter, not interlaced
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    chunk(&mut out, &ihdr);
    chunk(&mut out, b"IDAT");
    chunk(&mut out, b"IEND");
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

#[test]
fn huge_png_header_is_refused_before_decoding() {
    let tmp = tempfile::tempdir().unwrap();
    // 100 000 × 100 000 RGBA would be 40 GB: refused from the header.
    for (w, h) in [(100_000, 100_000), (4097, 16), (16, 4097)] {
        let path = tmp.path().join("big.png");
        std::fs::write(&path, png_header(w, h)).unwrap();
        refused(crate::own_textures::decode_png(&path), "sizes from 1 to 4096");
    }
    // At the limit, the header is accepted and the decoding fails on the missing pixels.
    let path = tmp.path().join("ok.png");
    std::fs::write(&path, png_header(4096, 4096)).unwrap();
    let e = crate::own_textures::decode_png(&path).unwrap_err();
    assert!(!e.contains("sizes from 1 to 4096"), "{e}");
}

#[test]
fn wav_with_few_bits_or_absurd_length() {
    let tmp = tempfile::tempdir().unwrap();
    // 12-bit samples (in 16-bit containers): read without a panic (the shift used to underflow);
    // when the reader accepts them, they are scaled up to 16 bits.
    let path = tmp.path().join("twelve.wav");
    std::fs::write(&path, wav_bytes(12, 0x7FFF_FFF0, 10, &[0x00, 0x80, 0xF0, 0xFF, 0, 0, 0x10, 0, 0xF0, 0x7F])).unwrap();
    if let Ok(d) = crate::own_sounds::decode(&path) {
        assert_eq!(d.samples.len(), 5);
    }
    // A data chunk that claims 2 GB over 4 bytes: an error, nothing sized by the claim.
    let wav = wav_bytes(16, 0x7FFF_FFF0, 0x7FFF_FF00, &[1, 0, 2, 0]);
    let path = tmp.path().join("long.wav");
    std::fs::write(&path, wav).unwrap();
    assert!(crate::own_sounds::decode(&path).is_err());
}

#[test]
fn ogg_decoding_stops_at_the_cap() {
    let mut samples = Vec::new();
    assert!(crate::own_sounds::push_capped(&mut samples, vec![1; 6], 10));
    assert!(!crate::own_sounds::push_capped(&mut samples, vec![2; 6], 10));
    assert!(crate::own_sounds::push_capped(&mut samples, vec![3; 4], 10));
    assert_eq!(samples, [1, 1, 1, 1, 1, 1, 3, 3, 3, 3]);
}

/// A mono 22 050 Hz PCM WAV file: `bits` per sample in 2-byte containers, the RIFF and data
/// chunk sizes as given (true or not), then `data`.
fn wav_bytes(bits: u16, riff_size: u32, data_size: u32, data: &[u8]) -> Vec<u8> {
    let mut wav = b"RIFF".to_vec();
    wav.extend(riff_size.to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes()); // PCM
    wav.extend(1u16.to_le_bytes()); // mono
    wav.extend(22050u32.to_le_bytes());
    wav.extend((22050u32 * 2).to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(bits.to_le_bytes());
    wav.extend(b"data");
    wav.extend(data_size.to_le_bytes());
    wav.extend(data);
    wav
}
