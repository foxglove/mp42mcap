use std::ptr;
use std::error::Error;
use std::ffi::CStr;
use ffmpeg_next as ffmpeg;
use ffmpeg_next::packet::Mut;

#[allow(non_camel_case_types)]
type AVBSFContext = *mut libc::c_void;
#[allow(non_camel_case_types)]
type AVBitStreamFilter = *mut libc::c_void;
#[allow(non_camel_case_types)]
type AVPacket = *mut libc::c_void;

extern "C" {
    fn av_bsf_get_by_name(name: *const libc::c_char) -> *const AVBitStreamFilter;
    fn av_bsf_alloc(filter: *const AVBitStreamFilter, ctx: *mut *mut AVBSFContext) -> libc::c_int;
    fn av_bsf_init(ctx: *mut AVBSFContext) -> libc::c_int;
    fn av_bsf_send_packet(ctx: *mut AVBSFContext, pkt: *mut AVPacket) -> libc::c_int;
    fn av_bsf_receive_packet(ctx: *mut AVBSFContext, pkt: *mut AVPacket) -> libc::c_int;
    fn av_bsf_free(ctx: *mut *mut AVBSFContext);
}

struct BSFContext {
    ctx: *mut AVBSFContext,
}

impl BSFContext {
    fn new(filter_name: &CStr) -> Result<Self, Box<dyn Error>> {
        unsafe {
            let bsf = av_bsf_get_by_name(filter_name.as_ptr());
            if bsf.is_null() {
                return Err(format!("Failed to get BSF: {}", filter_name.to_string_lossy()).into());
            }

            let mut ctx: *mut AVBSFContext = ptr::null_mut();
            let ret = av_bsf_alloc(bsf, &mut ctx);
            if ret < 0 {
                return Err(format!("Failed to allocate BSF context: {}", ret).into());
            }

            let ret = av_bsf_init(ctx);
            if ret < 0 {
                av_bsf_free(&mut ctx);
                return Err(format!("Failed to initialize BSF: {}", ret).into());
            }

            Ok(BSFContext { ctx })
        }
    }

    fn send_packet(&mut self, packet: &mut ffmpeg::Packet) -> Result<(), Box<dyn Error>> {
        unsafe {
            let ret = av_bsf_send_packet(self.ctx, packet.as_mut_ptr() as *mut _);
            if ret < 0 {
                return Err(format!("Failed to send packet to BSF: {}", ret).into());
            }
            Ok(())
        }
    }

    fn receive_packet(&mut self, packet: &mut ffmpeg::Packet) -> Result<bool, Box<dyn Error>> {
        unsafe {
            let ret = av_bsf_receive_packet(self.ctx, packet.as_mut_ptr() as *mut _);
            if ret == -libc::EAGAIN || ret == -libc::EINVAL /* EOF */ {
                Ok(false)
            } else if ret < 0 {
                Err(format!("Failed to receive packet from BSF: {}", ret).into())
            } else {
                Ok(true)
            }
        }
    }
}

impl Drop for BSFContext {
    fn drop(&mut self) {
        unsafe {
            av_bsf_free(&mut self.ctx);
        }
    }
}

pub fn apply_bsf(codec: ffmpeg::codec::Id, packet: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
    let filter_name = match codec {
        ffmpeg::codec::Id::H264 => "h264_mp4toannexb\0",
        ffmpeg::codec::Id::H265 | ffmpeg::codec::Id::HEVC => "hevc_mp4toannexb\0",
        other => return Err(format!("Unsupported codec {:?}", other).into()),
    };

    let mut bsf = BSFContext::new(CStr::from_bytes_with_nul(filter_name.as_bytes())?)?;
    let mut output = Vec::new();

    // Create input packet and copy data
    let mut in_pkt = ffmpeg::Packet::new(packet.len());
    unsafe {
        // Get the raw packet pointer and set data directly
        let raw_pkt = in_pkt.as_mut_ptr();
        (*raw_pkt).data = packet.as_ptr() as *mut _;
        (*raw_pkt).size = packet.len() as i32;
    }

    // Send packet to BSF
    bsf.send_packet(&mut in_pkt)?;

    // Receive filtered packets
    let mut out_pkt = ffmpeg::Packet::new(packet.len());
    while bsf.receive_packet(&mut out_pkt)? {
        if let Some(data) = out_pkt.data() {
            output.extend_from_slice(data);
        }
        out_pkt = ffmpeg::Packet::new(packet.len()); // Create new packet for next iteration
    }

    Ok(output)
}
