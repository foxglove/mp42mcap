fn main() {
    prost_build::Config::new()
        .compile_protos(&["proto/CompressedVideo.proto"], &["proto/"])
        .unwrap();
}
