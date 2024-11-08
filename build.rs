use std::env;
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    prost_build::Config::new()
        .file_descriptor_set_path(out_dir.join("foxglove_descriptor.bin"))
        .compile_protos(&["proto/CompressedVideo.proto"], &["proto/"])
        .unwrap();
}
