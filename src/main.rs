use clap::Parser;
use ffmpeg_next as ffmpeg;
use std::{io::Write, path::PathBuf};
use std::error::Error;
use std::borrow::Cow;

// Include generated protobuf code
pub mod foxglove {
        include!(concat!(env!("OUT_DIR"), "/foxglove.rs"));
}

use foxglove::CompressedVideo;
use prost::Message;
use prost_types;

use mcap::{Channel, Schema, Writer, records::MessageHeader};
use std::{collections::BTreeMap, fs::File, io::BufWriter};

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

    // Open MCAP file for writing
    let mut writer = Writer::new(
        BufWriter::new(File::create(&cli.output)?),
    )?;

    // Create video channel with schema
    let mcap_schema = Schema {
        name: String::from("foxglove.CompressedVideo"),
        encoding: String::from("protobuf"),
        data: Cow::Owned(include_bytes!(concat!(env!("OUT_DIR"), "/foxglove_descriptor.bin")).to_vec()),
    };

    let channel = Channel {
        topic: String::from("video"),
        message_encoding: String::from("protobuf"),
        schema: Some(mcap_schema.into()),
        metadata: BTreeMap::default(),
    };
    let channel_id = writer.add_channel(&channel)?;

    // Add sequence counter
    let mut sequence: u32 = 0;

    // Iterate over packets
    let mut frame = ffmpeg::frame::Video::empty();
    let mut packet_iter = input.packets();
    'outer: while let Some((stream, packet)) = packet_iter.next() {
        // Skip packets that aren't from our video stream
        if stream.index() != video_stream_index {
            continue;
        }

        // Get timestamp
        let timestamp_ns = (packet.pts().unwrap_or(0) * 1_000_000_000) as u64;

        // Send the packet to the decoder
        decoder.send_packet(&packet)?;

        // Receive all frames from this packet
        while decoder.receive_frame(&mut frame).is_ok() {
            print!(".");
            std::io::stdout().flush()?;

            // Create a VideoFrame message
            let message = CompressedVideo {
                frame_id: "video".to_string(),
                timestamp: Some(prost_types::Timestamp {
                    seconds: (timestamp_ns / 1_000_000_000) as i64,
                    nanos: (timestamp_ns % 1_000_000_000) as i32,
                }),
                data: frame.data(0).to_vec(),
                format: codec_format.to_string(),
            };

            // Serialize the protobuf message
            let encoded = message.encode_to_vec();

            // Write the encoded data to MCAP file
            writer.write_to_known_channel(
                &MessageHeader {
                    channel_id: channel_id,
                    sequence,
                    log_time: timestamp_ns,
                    publish_time: timestamp_ns,
                },
                &encoded,
            )?;

            // Increment sequence (wrap at u32::MAX)
            sequence = sequence.wrapping_add(1);

            // Stop after 10 frames while debugging
            if sequence >= 10 {
                break 'outer;
            }
        }
    }

    println!();

    // Close the MCAP file
    writer.finish()?;

    // Send EOF to cleanly close the decoder
    decoder.send_eof()?;

    Ok(())
}
