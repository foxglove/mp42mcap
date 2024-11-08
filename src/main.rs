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
    let mut input = ffmpeg::format::input(&cli.input)?;

    // Find the best video stream and store its index
    let video_stream_index = input
        .streams()
        .best(ffmpeg::media::Type::Video)
        .ok_or(ffmpeg::Error::StreamNotFound)?
        .index();

    // Get the decoder from the stream parameters
    let codec = ffmpeg::codec::context::Context::from_parameters(
        input.streams().best(ffmpeg::media::Type::Video).unwrap().parameters()
    )?;
    let mut decoder = codec.decoder().video()?;

    // Iterate over packets
    let mut frame = ffmpeg::frame::Video::empty();
    let mut packet_iter = input.packets();
    while let Some((stream, packet)) = packet_iter.next() {
        // Skip packets that aren't from our video stream
        if stream.index() != video_stream_index {
            continue;
        }

        // Send the packet to the decoder
        decoder.send_packet(&packet)?;

        // Receive all frames from this packet
        while decoder.receive_frame(&mut frame).is_ok() {
            println!("Processing frame: {}x{}", frame.width(), frame.height());
        }
    }

    // Flush the decoder
    decoder.send_eof()?;
    while decoder.receive_frame(&mut frame).is_ok() {
        println!("Processing frame: {}x{}", frame.width(), frame.height());
        // Process frame here
    }

    Ok(())
}
