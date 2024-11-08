use std::error::Error;

use ffmpeg_next as ffmpeg;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CodecType {
    H264,
    H265,
}

impl CodecType {
    const H264_NAL_SPS: u8 = 0x7;
    const H264_NAL_PPS: u8 = 0x8;
    const H264_NAL_SEI: u8 = 0x6;

    const H265_NAL_VPS: u8 = 32;
    const H265_NAL_SPS: u8 = 33;
    const H265_NAL_PPS: u8 = 34;
    const H265_NAL_SEI: u8 = 39;

    pub fn from_ffmpeg_id(id: ffmpeg::codec::Id) -> Result<Self, Box<dyn Error>> {
        match id {
            ffmpeg::codec::Id::H264 => Ok(CodecType::H264),
            ffmpeg::codec::Id::H265 | ffmpeg::codec::Id::HEVC => Ok(CodecType::H265),
            other => Err(format!("Unsupported codec {:?}", other).into()),
        }
    }

    pub fn format_str(&self) -> &'static str {
        match self {
            CodecType::H264 => "h264",
            CodecType::H265 => "h265",
        }
    }

    pub fn encoder_lib(&self) -> &'static str {
        match self {
            CodecType::H264 => "libx264",
            CodecType::H265 => "libx265",
        }
    }

    pub fn should_skip_nal(&self, nal_type: u8) -> bool {
        match self {
            CodecType::H264 => {
                matches!(
                    nal_type,
                    Self::H264_NAL_SPS | Self::H264_NAL_PPS | Self::H264_NAL_SEI
                )
            }
            CodecType::H265 => {
                matches!(
                    nal_type,
                    Self::H265_NAL_VPS
                        | Self::H265_NAL_SPS
                        | Self::H265_NAL_PPS
                        | Self::H265_NAL_SEI
                )
            }
        }
    }
}

pub struct ParameterSets {
    pub vps: Vec<u8>,
    pub sps: Vec<u8>,
    pub pps: Vec<u8>,
}

impl ParameterSets {
    const AVCC_HEADER_SIZE: usize = 5;
    const HVCC_HEADER_SIZE: usize = 22;

    pub fn parse(extradata: &[u8], codec: CodecType) -> Result<Self, Box<dyn Error>> {
        match codec {
            CodecType::H264 => Self::parse_avcc(extradata),
            CodecType::H265 => Self::parse_hvcc(extradata),
        }
    }

    fn parse_avcc(extradata: &[u8]) -> Result<Self, Box<dyn Error>> {
        if extradata.len() < Self::AVCC_HEADER_SIZE + 2 {
            return Err("AVCC header too short".into());
        }

        let mut offset = Self::AVCC_HEADER_SIZE;
        let mut sps_nals = Vec::new();
        let mut pps_nals = Vec::new();

        // Get SPS
        let num_sps = extradata[offset] & 0x1F;
        offset += 1;
        for _ in 0..num_sps {
            if offset + 2 > extradata.len() {
                return Err("Invalid SPS length".into());
            }
            let sps_size = ((extradata[offset] as usize) << 8) | (extradata[offset + 1] as usize);
            offset += 2;
            if offset + sps_size > extradata.len() {
                return Err("SPS data truncated".into());
            }
            sps_nals.extend_from_slice(&[0, 0, 0, 1]); // Add NAL start code
            sps_nals.extend_from_slice(&extradata[offset..offset + sps_size]);
            offset += sps_size;
        }

        // Get PPS
        if offset >= extradata.len() {
            return Err("Missing PPS".into());
        }
        let num_pps = extradata[offset];
        offset += 1;
        for _ in 0..num_pps {
            if offset + 2 > extradata.len() {
                return Err("Invalid PPS length".into());
            }
            let pps_size = ((extradata[offset] as usize) << 8) | (extradata[offset + 1] as usize);
            offset += 2;
            if offset + pps_size > extradata.len() {
                return Err("PPS data truncated".into());
            }
            pps_nals.extend_from_slice(&[0, 0, 0, 1]); // Add NAL start code
            pps_nals.extend_from_slice(&extradata[offset..offset + pps_size]);
            offset += pps_size;
        }

        if sps_nals.is_empty() || pps_nals.is_empty() {
            return Err("Missing required parameter sets".into());
        }

        Ok(Self {
            vps: Vec::new(), // H.264 doesn't use VPS
            sps: sps_nals,
            pps: pps_nals,
        })
    }

    fn parse_hvcc(extradata: &[u8]) -> Result<Self, Box<dyn Error>> {
        if extradata.len() < Self::HVCC_HEADER_SIZE + 1 {
            return Err("HVCC header too short".into());
        }

        let mut vps_nals = Vec::new();
        let mut sps_nals = Vec::new();
        let mut pps_nals = Vec::new();

        let mut offset = Self::HVCC_HEADER_SIZE;

        let num_arrays = extradata[offset];
        offset += 1;

        for _ in 0..num_arrays {
            if offset + 3 > extradata.len() {
                break;
            }

            let nal_type = extradata[offset] & 0x3F;
            let num_nals =
                ((extradata[offset + 1] as usize) << 8) | (extradata[offset + 2] as usize);
            offset += 3;

            for _ in 0..num_nals {
                if offset + 2 > extradata.len() {
                    break;
                }

                let nal_size =
                    ((extradata[offset] as usize) << 8) | (extradata[offset + 1] as usize);
                offset += 2;

                if offset + nal_size > extradata.len() {
                    break;
                }

                let nal_data = &[0, 0, 0, 1];
                match nal_type {
                    32 => {
                        vps_nals.extend_from_slice(nal_data);
                        vps_nals.extend_from_slice(&extradata[offset..offset + nal_size]);
                    }
                    33 => {
                        sps_nals.extend_from_slice(nal_data);
                        sps_nals.extend_from_slice(&extradata[offset..offset + nal_size]);
                    }
                    34 => {
                        pps_nals.extend_from_slice(nal_data);
                        pps_nals.extend_from_slice(&extradata[offset..offset + nal_size]);
                    }
                    _ => {}
                }
                offset += nal_size;
            }
        }

        if sps_nals.is_empty() || pps_nals.is_empty() {
            return Err("Missing required HEVC parameter sets".into());
        }

        Ok(Self {
            vps: vps_nals,
            sps: sps_nals,
            pps: pps_nals,
        })
    }

    pub fn write_to(&self, codec: CodecType, buffer: &mut Vec<u8>) {
        if codec == CodecType::H265 {
            buffer.extend_from_slice(&self.vps);
        }
        buffer.extend_from_slice(&self.sps);
        buffer.extend_from_slice(&self.pps);
    }

    pub fn validate(&self, codec: CodecType) -> Result<(), Box<dyn Error>> {
        if self.sps.is_empty() || self.pps.is_empty() {
            return Err("Missing required parameter sets".into());
        }
        if codec == CodecType::H265 && self.vps.is_empty() {
            return Err("Missing required VPS for H.265".into());
        }
        Ok(())
    }
}

pub fn convert_to_annex_b(data: &[u8], codec: CodecType) -> Vec<u8> {
    let mut converted = Vec::new();
    let mut pos = 0;

    while pos < data.len() {
        if pos + 4 > data.len() {
            break;
        }
        let nal_size =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;

        let nal_type = if pos + 4 + 1 <= data.len() {
            match codec {
                CodecType::H264 => data[pos + 4] & 0x1F,
                CodecType::H265 => (data[pos + 4] >> 1) & 0x3F,
            }
        } else {
            0
        };

        pos += 4;

        if !codec.should_skip_nal(nal_type) {
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

pub fn extract_extradata(params: &ffmpeg::codec::Parameters) -> Result<&[u8], Box<dyn Error>> {
    unsafe {
        let ptr = params.as_ptr();
        if (*ptr).extradata.is_null() {
            return Err("No codec extradata found".into());
        }
        Ok(std::slice::from_raw_parts(
            (*ptr).extradata,
            (*ptr).extradata_size as usize,
        ))
    }
}

pub struct VideoConverter {
    codec_type: CodecType,
    decoder: ffmpeg::decoder::Video,
    parameter_sets: ParameterSets,
    time_base_num: i32,
    time_base_den: i32,
    frame_packets: Vec<Vec<u8>>,
    last_timestamp: u64,
    last_progress: u64,
}

impl VideoConverter {
    pub fn new(
        input_path: &std::path::Path,
    ) -> Result<(Self, ffmpeg::format::context::Input), Box<dyn Error>> {
        let input = ffmpeg::format::input(input_path)?;
        let video_stream = input
            .streams()
            .best(ffmpeg::media::Type::Video)
            .ok_or(ffmpeg::Error::StreamNotFound)?;

        let codec = ffmpeg::codec::context::Context::from_parameters(video_stream.parameters())?;
        let codec_type = CodecType::from_ffmpeg_id(codec.id())?;
        let decoder = codec.decoder().video()?;

        // Create binding to extend lifetime of parameters
        let params = video_stream.parameters();
        let extradata = extract_extradata(&params)?;
        let parameter_sets = ParameterSets::parse(extradata, codec_type)?;
        parameter_sets.validate(codec_type)?;

        let time_base = video_stream.time_base();

        Ok((
            Self {
                codec_type,
                decoder,
                parameter_sets,
                time_base_num: time_base.numerator(),
                time_base_den: time_base.denominator(),
                frame_packets: Vec::new(),
                last_timestamp: u64::MAX,
                last_progress: 0,
            },
            input,
        ))
    }

    pub fn send_packet(&mut self, packet: &ffmpeg::Packet) -> Result<(), ffmpeg::Error> {
        self.decoder.send_packet(packet)
    }

    pub fn receive_frame(&mut self, frame: &mut ffmpeg::frame::Video) -> Result<(), ffmpeg::Error> {
        self.decoder.receive_frame(frame)
    }

    pub fn send_eof(&mut self) -> Result<(), ffmpeg::Error> {
        self.decoder.send_eof()
    }

    pub fn process_packet(
        &mut self,
        packet: &ffmpeg::Packet,
        is_first: bool,
    ) -> Result<(), Box<dyn Error>> {
        if let Some(data) = packet.data() {
            if !data.is_empty() {
                let pts = packet.pts().ok_or("Missing PTS")?;
                let dts = packet.dts().ok_or("Missing DTS")?;

                if pts != dts {
                    return Err(format!(
                        "This video contains B-frames or reordered frames (PTS={}, DTS={}). \
                        Please re-encode the video without B-frames using: \
                        ffmpeg -i <input> -c:v {} -bf 0 output.mp4",
                        pts,
                        dts,
                        self.codec_type.encoder_lib()
                    )
                    .into());
                }

                if is_first || packet.is_key() {
                    let mut frame_data = Vec::new();
                    self.parameter_sets
                        .write_to(self.codec_type, &mut frame_data);
                    let converted = convert_to_annex_b(data, self.codec_type);
                    frame_data.extend_from_slice(&converted);
                    self.frame_packets.push(frame_data);
                } else {
                    let converted = convert_to_annex_b(data, self.codec_type);
                    self.frame_packets.push(converted);
                }
            }
        }
        Ok(())
    }

    pub fn get_timestamp(&self, pts: i64) -> u64 {
        (pts as f64 * self.time_base_num as f64 / self.time_base_den as f64 * 1_000_000_000.0)
            as u64
    }

    pub fn check_timestamp(&mut self, timestamp_ns: u64) -> Result<(), Box<dyn Error>> {
        if timestamp_ns <= self.last_timestamp && self.last_timestamp != u64::MAX {
            return Err(format!(
                "Non-monotonic or duplicate timestamp detected! Current: {}ns, Last: {}ns",
                timestamp_ns, self.last_timestamp
            )
            .into());
        }
        self.last_timestamp = timestamp_ns;
        Ok(())
    }

    pub fn update_progress(&mut self, timestamp_ns: u64) -> bool {
        if timestamp_ns >= self.last_progress + 1_000_000_000 {
            self.last_progress = timestamp_ns;
            true
        } else {
            false
        }
    }

    pub fn take_frame_data(&mut self) -> Vec<u8> {
        let mut frame_data = Vec::new();
        for packet_data in self.frame_packets.iter() {
            frame_data.extend_from_slice(packet_data);
        }
        self.frame_packets.clear();
        frame_data
    }

    pub fn format_str(&self) -> &'static str {
        self.codec_type.format_str()
    }
}
