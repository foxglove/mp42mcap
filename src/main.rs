use clap::Parser;
use ffmpeg_next as ffmpeg;
use std::path::PathBuf;

/// Convert MP4 files to MCAP format
#[derive(Parser)]
#[command(name = env!("CARGO_PKG_NAME"))]
#[command(author = env!("CARGO_PKG_AUTHORS"))]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "Converts MP4 videos to MCAP", long_about = None)]
#[command(arg_required_else_help = true)]
struct Cli {
    /// Input MP4 file
    #[arg(value_name = "INPUT")]
    input: PathBuf,

    /// Output MCAP file
    #[arg(value_name = "OUTPUT")]
    output: PathBuf,
}

fn main() -> Result<(), ffmpeg::Error> {
    let cli = Cli::parse();

    println!("Converting {:?} to {:?}", cli.input, cli.output);

    // Initialize FFmpeg
    ffmpeg::init()?;

    // Open the input file
    let input = ffmpeg::format::input(&cli.input)?;

    // Find the best video stream
    let video_stream = input
        .streams()
        .best(ffmpeg::media::Type::Video)
        .ok_or(ffmpeg::Error::StreamNotFound)?;

    // Get the decoder
    let codec = ffmpeg::codec::context::Context::from_parameters(video_stream.parameters())?;
    let mut decoder = codec.decoder().video()?;

    // Now you can receive frames
    let mut frame = ffmpeg::frame::Video::empty();

    while let Ok(_) = decoder.receive_frame(&mut frame) {
        // Here you will process each frame and write to MCAP
        // frame.data(0) contains the raw pixel data
        println!("Processing frame: {}x{}", frame.width(), frame.height());
    }

    Ok(())
}
