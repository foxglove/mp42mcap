use std::env;
use std::path::PathBuf;

fn main() {
    // Link against specific FFmpeg library versions
    println!("cargo:rustc-link-search=native=/opt/homebrew/Cellar/ffmpeg/7.1_3/lib");
    println!("cargo:rustc-link-lib=dylib=avcodec");
    println!("cargo:rustc-link-lib=dylib=avformat");
    println!("cargo:rustc-link-lib=dylib=avutil");
    println!("cargo:rustc-link-lib=dylib=avfilter");
    println!("cargo:rustc-link-lib=dylib=swscale");

    // Add FFmpeg include directory
    println!("cargo:rustc-env=CFLAGS=-I/opt/homebrew/Cellar/ffmpeg/7.1_3/include");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    prost_build::Config::new()
        .file_descriptor_set_path(out_dir.join("foxglove_descriptor.bin"))
        .compile_protos(&["proto/CompressedVideo.proto"], &["proto/"])
        .unwrap();
}
