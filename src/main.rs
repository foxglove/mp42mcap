use clap::Parser;
use ffmpeg_next as ffmpeg;
use std::borrow::Cow;
use std::error::Error;
use std::{io::Write, path::PathBuf};

// Include generated protobuf code
pub mod foxglove {
    include!(concat!(env!("OUT_DIR"), "/foxglove.rs"));
}

use foxglove::CompressedVideo;
use prost::Message;
use prost_types;

use mcap::{records::MessageHeader, Channel, Schema, Writer};
use std::{collections::BTreeMap, fs::File, io::BufWriter};

/// Converts a video packet from AVCC/HVCC format (length-prefixed) to Annex B format (start code-prefixed)
fn convert_to_annex_b(data: &[u8], codec_format: &str) -> Vec<u8> {
    let mut converted = Vec::new();
    let mut pos = 0;

    while pos < data.len() {
        if pos + 4 > data.len() {
            break;
        }
        let nal_size =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;

        // Get NAL type
        let nal_type = if pos + 4 + 1 <= data.len() {
            match codec_format {
                "h264" => data[pos + 4] & 0x1F,        // Last 5 bits for H.264
                "h265" => (data[pos + 4] >> 1) & 0x3F, // Bits 1-6 for H.265
                _ => 0,
            }
        } else {
            0
        };

        pos += 4;

        // Skip SPS, PPS, and SEI NALs since we're manually handling them
        // 7 = SPS, 8 = PPS, 6 = SEI
        if nal_type != 0x7 && nal_type != 0x8 && nal_type != 0x6 {
            converted.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
            if pos + nal_size <= data.len() {
                converted.extend_from_slice(&data[pos..pos + nal_size]);
            }
        }

        if pos + nal_size <= data.len() {
            pos += nal_size;
        } else {
            break;
        }
    }

    converted
}

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
        input
            .streams()
            .best(ffmpeg::media::Type::Video)
            .unwrap()
            .parameters(),
    )?;

    // Check if codec is supported and get format string
    let codec_id = codec.id();
    let codec_format = match codec_id {
        ffmpeg::codec::Id::H264 => "h264",
        ffmpeg::codec::Id::H265 => "h265",
        ffmpeg::codec::Id::HEVC => "h265",
        other => return Err(format!("Unsupported codec {:?}", other).into()),
    };

    // Get the video stream parameters
    let params = input
        .streams()
        .best(ffmpeg::media::Type::Video)
        .unwrap()
        .parameters();

    // Extract SPS/PPS from codec parameters
    let extradata = unsafe {
        let ptr = params.as_ptr();
        if (*ptr).extradata.is_null() {
            panic!("No codec extradata found");
        }
        std::slice::from_raw_parts((*ptr).extradata, (*ptr).extradata_size as usize)
    };

    // Create the decoder after getting all the info we need
    let mut decoder = codec.decoder().video()?;

    // Parse AVCC format extradata to get SPS/PPS
    let mut sps_nals = Vec::new();
    let mut pps_nals = Vec::new();

    // AVCC format: [1 byte version][1 byte profile][1 byte compat][1 byte level][6 bits reserved + 2 bits NALsize-1][1 byte num SPS][2 bytes SPS size][SPS data][1 byte num PPS][2 bytes PPS size][PPS data]
    let mut offset = 5; // Skip AVCC header

    // Get SPS
    let num_sps = extradata[offset] & 0x1F;
    offset += 1;
    for _ in 0..num_sps {
        let sps_size = ((extradata[offset] as usize) << 8) | (extradata[offset + 1] as usize);
        offset += 2;
        sps_nals.extend_from_slice(&[0, 0, 0, 1]); // Add NAL start code
        sps_nals.extend_from_slice(&extradata[offset..offset + sps_size]);
        offset += sps_size;
    }

    // Get PPS
    let num_pps = extradata[offset];
    offset += 1;
    for _ in 0..num_pps {
        let pps_size = ((extradata[offset] as usize) << 8) | (extradata[offset + 1] as usize);
        offset += 2;
        pps_nals.extend_from_slice(&[0, 0, 0, 1]); // Add NAL start code
        pps_nals.extend_from_slice(&extradata[offset..offset + pps_size]);
        offset += pps_size;
    }

    // Open MCAP file for writing
    let mut writer = Writer::new(BufWriter::new(File::create(&cli.output)?))?;

    // Create video channel with schema
    let schema = Schema {
        name: String::from("foxglove.CompressedVideo"),
        encoding: String::from("protobuf"),
        data: Cow::Owned(
            include_bytes!(concat!(env!("OUT_DIR"), "/foxglove_descriptor.bin")).to_vec(),
        ),
    };
    let channel = Channel {
        topic: String::from("video"),
        message_encoding: String::from("protobuf"),
        schema: Some(schema.into()),
        metadata: BTreeMap::default(),
    };
    let channel_id = writer.add_channel(&channel)?;

    // Add sequence counter
    let mut sequence: u32 = 0;

    // Get the video stream for time base information
    let video_stream = input.streams().best(ffmpeg::media::Type::Video).unwrap();
    let time_base = video_stream.time_base();

    // Create the packet iterator
    let mut frame = ffmpeg::frame::Video::empty();
    let mut packet_iter = input.packets();
    let mut frame_packets: Vec<Vec<u8>> = Vec::new();
    let mut first_frame = true;

    let mut last_timestamp = u64::MAX; // Initialize to max value so first frame always passes
    let mut last_progress = 0u64; // Track last progress update in nanoseconds

    while let Some((stream, packet)) = packet_iter.next() {
        // Skip packets that aren't from our video stream
        if stream.index() != video_stream_index {
            continue;
        }

        // Get timestamp
        let pts = packet.pts().unwrap_or(0);
        let dts = packet.dts().unwrap_or(0);
        let timestamp_ns = (pts as f64 * time_base.numerator() as f64
            / time_base.denominator() as f64
            * 1_000_000_000.0) as u64;

        // Convert AVCC/HVCC to Annex B
        if let Some(data) = packet.data() {
            if !data.is_empty() {
                let pts = packet.pts().ok_or("Missing PTS")?;
                let dts = packet.dts().ok_or("Missing DTS")?;

                if pts != dts {
                    return Err(format!(
                        "This video contains B-frames or reordered frames (PTS={}, DTS={}). \
                        Please re-encode the video without B-frames using: \
                        ffmpeg -i {} -c:v libx264 -bf 0 output.mp4",
                        pts,
                        dts,
                        cli.input.display()
                    )
                    .into());
                }

                if first_frame {
                    first_frame = false;
                    let mut frame_data = Vec::new();
                    // Add SPS and PPS first
                    frame_data.extend_from_slice(&sps_nals);
                    frame_data.extend_from_slice(&pps_nals);
                    // Then convert and add the packet data
                    let converted = convert_to_annex_b(data, codec_format);
                    frame_data.extend_from_slice(&converted);
                    frame_packets.push(frame_data);
                } else if packet.is_key() {
                    let mut frame_data = Vec::new();
                    // Add SPS and PPS first
                    frame_data.extend_from_slice(&sps_nals);
                    frame_data.extend_from_slice(&pps_nals);
                    // Then convert and add the packet data
                    let converted = convert_to_annex_b(data, codec_format);
                    frame_data.extend_from_slice(&converted);
                    frame_packets.push(frame_data);
                } else {
                    let converted = convert_to_annex_b(data, codec_format);
                    frame_packets.push(converted);
                }
            }
        }

        // Send the packet to the decoder
        decoder.send_packet(&packet)?;

        // Receive frame if ready
        match decoder.receive_frame(&mut frame) {
            Ok(_) => {
                // Print . for every 1s of video
                if timestamp_ns >= last_progress + 1_000_000_000 {
                    print!(".");
                    std::io::stdout().flush()?;
                    last_progress = timestamp_ns;
                }

                // Create a single buffer for the entire frame
                let mut frame_data = Vec::new();
                for packet_data in frame_packets.iter() {
                    frame_data.extend_from_slice(packet_data);
                }

                // Check for monotonic timestamps before writing
                if timestamp_ns <= last_timestamp && last_timestamp != u64::MAX {
                    return Err(format!(
                        "Non-monotonic or duplicate timestamp detected! Current: {}ns, Last: {}ns, PTS: {}, DTS: {}",
                        timestamp_ns, last_timestamp, pts, dts
                    ).into());
                }
                last_timestamp = timestamp_ns;

                // Create and write the message
                let message = CompressedVideo {
                    frame_id: "video".to_string(),
                    timestamp: Some(prost_types::Timestamp {
                        seconds: (timestamp_ns / 1_000_000_000) as i64,
                        nanos: (timestamp_ns % 1_000_000_000) as i32,
                    }),
                    data: frame_data,
                    format: codec_format.to_string(),
                };

                // Clear packets buffer for next frame
                frame_packets.clear();

                // Serialize and write the protobuf message
                let encoded = message.encode_to_vec();
                writer.write_to_known_channel(
                    &MessageHeader {
                        channel_id,
                        sequence,
                        log_time: timestamp_ns,
                        publish_time: timestamp_ns,
                    },
                    &encoded,
                )?;

                // Increment sequence
                sequence = sequence.wrapping_add(1);
                if sequence >= 100 {
                    break;
                }
            }
            Err(ffmpeg::Error::Other {
                errno: ffmpeg::error::EAGAIN,
            }) => {
                continue;
            }
            Err(e) => return Err(e.into()),
        }
    }

    println!();

    // Close the MCAP file
    writer.finish()?;

    // Send EOF to cleanly close the decoder
    decoder.send_eof()?;

    Ok(())
}
