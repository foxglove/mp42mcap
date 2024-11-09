use std::{
    borrow::Cow,
    collections::BTreeMap,
    error::Error,
    fs::File,
    io::{BufWriter, Write},
    path::PathBuf,
    time::Instant,
};

use clap::Parser;
use ffmpeg_next as ffmpeg;
use mcap::{records::MessageHeader, Channel, Schema, Writer};
use prost::Message;

pub mod foxglove {
    include!(concat!(env!("OUT_DIR"), "/foxglove.rs"));
}
use foxglove::CompressedVideo;

mod codec;
use codec::VideoConverter;

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

    /// Topic name for the video messages
    #[arg(long, default_value = "video")]
    topic: String,

    /// Frame ID for the video messages
    #[arg(long, default_value = "video")]
    frame_id: String,
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    let start_time = Instant::now();
    println!("Converting {:?} to {:?}", cli.input, cli.output);

    ffmpeg::init()?;

    let (mut converter, mut input) = VideoConverter::new(&cli.input)?;
    let video_stream_index = input
        .streams()
        .best(ffmpeg::media::Type::Video)
        .ok_or(ffmpeg::Error::StreamNotFound)?
        .index();

    let mut writer = Writer::new(BufWriter::new(File::create(&cli.output)?))?;
    let channel_id = setup_mcap_channel(&mut writer, &cli.topic)?;

    let mut sequence: u32 = 0;
    let mut frame = ffmpeg::frame::Video::empty();
    let first_frame = true;

    let packet_iter = input.packets();
    for (stream, packet) in packet_iter {
        if stream.index() != video_stream_index {
            continue;
        }

        let timestamp_ns = converter.get_timestamp(packet.pts().unwrap_or(0));
        converter.process_packet(&packet, first_frame)?;
        converter.send_packet(&packet)?;

        match converter.receive_frame(&mut frame) {
            Ok(_) => {
                if converter.update_progress(timestamp_ns) {
                    print!(".");
                    std::io::stdout().flush()?;
                }

                converter.check_timestamp(timestamp_ns)?;

                let message = CompressedVideo {
                    frame_id: cli.frame_id.clone(),
                    timestamp: Some(prost_types::Timestamp {
                        seconds: (timestamp_ns / 1_000_000_000) as i64,
                        nanos: (timestamp_ns % 1_000_000_000) as i32,
                    }),
                    data: converter.take_frame_data(),
                    format: converter.format_str().to_string(),
                };

                let encoded = message.encode_to_vec();
                writer.write_to_known_channel(
                    &MessageHeader {
                        channel_id: channel_id.try_into().unwrap(),
                        sequence,
                        log_time: timestamp_ns,
                        publish_time: timestamp_ns,
                    },
                    &encoded,
                )?;

                sequence = sequence.wrapping_add(1);
            }
            Err(ffmpeg::Error::Other {
                errno: ffmpeg::error::EAGAIN,
            }) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    writer.finish()?;
    converter.send_eof()?;

    println!(
        "\nCompleted in {:.3} seconds",
        start_time.elapsed().as_secs_f64()
    );

    Ok(())
}

fn setup_mcap_channel(
    writer: &mut Writer<BufWriter<File>>,
    topic: &str,
) -> Result<u64, Box<dyn Error>> {
    let schema = Schema {
        name: String::from("foxglove.CompressedVideo"),
        encoding: String::from("protobuf"),
        data: Cow::Owned(
            include_bytes!(concat!(env!("OUT_DIR"), "/foxglove_descriptor.bin")).to_vec(),
        ),
    };
    let channel = Channel {
        topic: topic.to_string(),
        message_encoding: String::from("protobuf"),
        schema: Some(schema.into()),
        metadata: BTreeMap::default(),
    };
    Ok(writer.add_channel(&channel)?.into())
}
