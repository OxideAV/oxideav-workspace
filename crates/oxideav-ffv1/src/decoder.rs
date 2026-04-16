//! FFV1 packet decoder.
//!
//! Each compressed packet holds one frame (keyframe flag: 1 bit) followed
//! by one or more slices. Our decoder only implements our simple profile
//! (single-slice, 8-bit YCbCr 4:2:0 or 4:4:4, coder_type=1). Foreign streams
//! producing multiple slices will be rejected with `Error::Unsupported`.

use oxideav_codec::Decoder;
use oxideav_core::frame::VideoPlane;
use oxideav_core::{
    CodecId, CodecParameters, Error, Frame, Packet, PixelFormat, Result, VideoFrame,
};

use crate::config::ConfigRecord;
use crate::range_coder::RangeDecoder;
use crate::slice::{decode_slice, PlaneGeom};

pub fn make_decoder(params: &CodecParameters) -> Result<Box<dyn Decoder>> {
    let config = if params.extradata.is_empty() {
        // Allow extradata-less streams at our own default.
        let yuv444 = matches!(params.pixel_format, Some(PixelFormat::Yuv444P));
        ConfigRecord::new_simple(yuv444)
    } else {
        ConfigRecord::parse(&params.extradata)?
    };
    let width = params
        .width
        .ok_or_else(|| Error::invalid("FFV1 decoder: missing width"))?;
    let height = params
        .height
        .ok_or_else(|| Error::invalid("FFV1 decoder: missing height"))?;
    Ok(Box::new(Ffv1Decoder {
        codec_id: params.codec_id.clone(),
        config,
        width,
        height,
        pending: None,
        eof: false,
    }))
}

struct Ffv1Decoder {
    codec_id: CodecId,
    config: ConfigRecord,
    width: u32,
    height: u32,
    pending: Option<Packet>,
    eof: bool,
}

impl Decoder for Ffv1Decoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }

    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        if self.pending.is_some() {
            return Err(Error::other(
                "FFV1 decoder: receive_frame must be called before another packet",
            ));
        }
        self.pending = Some(packet.clone());
        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Frame> {
        let Some(pkt) = self.pending.take() else {
            return if self.eof {
                Err(Error::Eof)
            } else {
                Err(Error::NeedMore)
            };
        };
        let vf = decode_packet(&self.config, &pkt, self.width, self.height)?;
        Ok(Frame::Video(vf))
    }

    fn flush(&mut self) -> Result<()> {
        self.eof = true;
        Ok(())
    }
}

fn decode_packet(
    config: &ConfigRecord,
    pkt: &Packet,
    width: u32,
    height: u32,
) -> Result<VideoFrame> {
    if config.num_h_slices != 1 || config.num_v_slices != 1 {
        return Err(Error::unsupported("FFV1: multi-slice decode"));
    }
    let data = &pkt.data;
    if data.is_empty() {
        return Err(Error::invalid("FFV1 decode: empty packet"));
    }

    // A top-level range-coded "picture header" covers the keyframe bit. Its
    // state is a single byte. We then switch to decoding the slice data
    // directly; since our single-slice simple profile puts the slice
    // immediately after the keyframe bit (using range-coded state), we
    // reuse the remaining buffer as the slice bytes. This mirrors how
    // FFmpeg reads a v3 keyframe packet.
    let mut kf_dec = RangeDecoder::new(data);
    let mut kf_state = 128u8;
    let _keyframe = kf_dec.get_rac(&mut kf_state);

    // Map the pixel format from the config record.
    let pix_fmt = if config.is_yuv420() {
        PixelFormat::Yuv420P
    } else if config.is_yuv444() {
        PixelFormat::Yuv444P
    } else {
        return Err(Error::unsupported("FFV1: unsupported chroma subsampling"));
    };

    let (cw, ch) = match pix_fmt {
        PixelFormat::Yuv420P => (width.div_ceil(2), height.div_ceil(2)),
        PixelFormat::Yuv444P => (width, height),
        _ => unreachable!(),
    };

    // After the keyframe bit was decoded, the remainder of the bytestream
    // is the slice payload (since we emit a single slice). Our single-slice
    // encoder records the slice size in the last 3 bytes — decode_slice
    // strips them.
    let slice_start = kf_dec.position();
    let slice_bytes = &data[slice_start..];
    let y_geom = PlaneGeom { width, height };
    let c_geom = PlaneGeom {
        width: cw,
        height: ch,
    };
    let decoded = decode_slice(slice_bytes, y_geom, Some(c_geom))?;

    let y_plane = VideoPlane {
        stride: width as usize,
        data: decoded.y,
    };
    let u_plane = VideoPlane {
        stride: cw as usize,
        data: decoded.u.unwrap_or_default(),
    };
    let v_plane = VideoPlane {
        stride: cw as usize,
        data: decoded.v.unwrap_or_default(),
    };

    Ok(VideoFrame {
        format: pix_fmt,
        width,
        height,
        pts: pkt.pts,
        time_base: pkt.time_base,
        planes: vec![y_plane, u_plane, v_plane],
    })
}
