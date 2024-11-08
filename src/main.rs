use clap::Parser;
use ffmpeg_next as ffmpeg;
use std::path::PathBuf;
use std::error::Error;

// Include generated protobuf code
pub mod foxglove {
        include!(concat!(env!("OUT_DIR"), "/foxglove.rs"));
}

use foxglove::CompressedVideo;
use prost::Message;
use prost_types;

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

fn main() -> Result<(), Box<dyn Error>> {
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

    // Check if codec is supported and get format string
    let codec_format = match codec.id() {
        ffmpeg::codec::Id::H264 => "h264",
        ffmpeg::codec::Id::H265 => "h265",
        ffmpeg::codec::Id::HEVC => "h265",
        other => return Err(format!("Unsupported codec {:?}", other).into()),
    };

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

            // Create a VideoFrame message
            let message = CompressedVideo {
                frame_id: "video".to_string(),
                timestamp: Some(prost_types::Timestamp {
                    seconds: packet.pts().unwrap_or(0) / 1000,
                    nanos: ((packet.pts().unwrap_or(0) % 1000) * 1_000_000) as i32,
                }),
                data: frame.data(0).to_vec(),
                format: codec_format.to_string(),
            };

            // Serialize the protobuf message
            let _encoded = message.encode_to_vec();

            // TODO: Write the encoded data to your MCAP file
            // You'll need to implement the MCAP writing logic here
        }
    }

    // Send EOF to cleanly close the decoder
    decoder.send_eof()?;

    Ok(())
}
