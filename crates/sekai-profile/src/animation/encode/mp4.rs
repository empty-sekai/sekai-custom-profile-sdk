//! H.264 MP4 output: encoder configuration, the libx264 binding and the AVC MP4 muxer.

use serde::{Deserialize, Serialize};

use super::yuv::{rgba_to_yuv420p_into, rgba_to_yuv420p_macroblocks_into};
use super::{checked_frame, output_budget_exceeded, AnimationEncodeTelemetry};
use crate::animation::compositor::FrameCompositeUpdate;
use crate::animation::AnimationEncodeSpec;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum H264YuvConverter {
    Auto,
    Scalar,
    Avx512,
}

impl H264YuvConverter {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "auto" => Ok(Self::Auto),
            "scalar" => Ok(Self::Scalar),
            "avx512" => Ok(Self::Avx512),
            _ => Err(format!("unsupported H.264 YUV converter: {value}")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Scalar => "scalar",
            Self::Avx512 => "avx512",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct H264EncoderConfig {
    pub qp: u8,
    pub yuv_converter: H264YuvConverter,
    pub require_avx512: bool,
    pub validate_avx512: bool,
    pub encoder_revision: String,
}

impl Default for H264EncoderConfig {
    fn default() -> Self {
        Self {
            qp: 18,
            yuv_converter: H264YuvConverter::Auto,
            require_avx512: false,
            validate_avx512: false,
            encoder_revision: "direct-libx264-lazy-font-mp4-v9".into(),
        }
    }
}

impl H264EncoderConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.qp > 51 {
            return Err(format!("invalid H.264 QP: {}", self.qp));
        }
        if (self.require_avx512 || self.validate_avx512)
            && self.yuv_converter == H264YuvConverter::Scalar
        {
            return Err("H.264 AVX-512 was required but the scalar converter was selected".into());
        }
        if self.encoder_revision.is_empty()
            || !self
                .encoder_revision
                .bytes()
                .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'.' | b'_' | b'-'))
        {
            return Err("H.264 encoder revision must use [A-Za-z0-9._-]".into());
        }
        Ok(())
    }

    pub fn cache_identity(&self) -> Result<String, String> {
        self.validate()?;
        Ok(format!(
            "h264-v2-{}-qp-{}-yuv-{}-require512-{}",
            self.encoder_revision,
            self.qp,
            self.yuv_converter.as_str(),
            self.require_avx512
        ))
    }
}

pub(super) fn encode_mp4<F>(
    spec: &AnimationEncodeSpec,
    expected: usize,
    config: &H264EncoderConfig,
    telemetry: &mut AnimationEncodeTelemetry,
    frame_at: &mut F,
) -> Result<Vec<u8>, String>
where
    F: FnMut(u32) -> Result<Vec<u8>, String>,
{
    encode_mp4_into(spec, expected, config, telemetry, |index, rgba| {
        *rgba = checked_frame(frame_at, index, expected)?;
        Ok(FrameCompositeUpdate::Full)
    })
}

#[repr(C)]
struct NativeX264Encoder {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn allium_x264_create(
        width: std::ffi::c_int,
        height: std::ffi::c_int,
        fps: std::ffi::c_int,
        crf: std::ffi::c_int,
    ) -> *mut NativeX264Encoder;
    fn allium_x264_encode(
        encoder: *mut NativeX264Encoder,
        yuv420p: *const u8,
        mb_info: *const u8,
        pts: i64,
        output: *mut *const u8,
        output_size: *mut usize,
    ) -> std::ffi::c_int;
    fn allium_x264_flush(
        encoder: *mut NativeX264Encoder,
        output: *mut *const u8,
        output_size: *mut usize,
    ) -> std::ffi::c_int;
    fn allium_x264_delayed_frames(encoder: *const NativeX264Encoder) -> std::ffi::c_int;
    fn allium_x264_destroy(encoder: *mut NativeX264Encoder);
}

struct DirectX264Encoder(*mut NativeX264Encoder);

impl DirectX264Encoder {
    fn new(spec: &AnimationEncodeSpec, config: &H264EncoderConfig) -> Result<Self, String> {
        let encoder = unsafe {
            allium_x264_create(
                spec.width as std::ffi::c_int,
                spec.height as std::ffi::c_int,
                spec.fps as std::ffi::c_int,
                config.qp as std::ffi::c_int,
            )
        };
        if encoder.is_null() {
            Err("initialize direct libx264 encoder failed".into())
        } else {
            Ok(Self(encoder))
        }
    }

    fn encode(
        &mut self,
        yuv: &[u8],
        mb_info: &[u8],
        pts: i64,
        output: &mut Vec<u8>,
    ) -> Result<(), String> {
        let mut data = std::ptr::null();
        let mut size = 0usize;
        let encoded = unsafe {
            allium_x264_encode(
                self.0,
                yuv.as_ptr(),
                mb_info.as_ptr(),
                pts,
                &mut data,
                &mut size,
            )
        };
        if encoded < 0 || (size != 0 && data.is_null()) {
            return Err(format!("direct libx264 frame {pts} failed"));
        }
        if size != 0 {
            output.extend_from_slice(unsafe { std::slice::from_raw_parts(data, size) });
        }
        Ok(())
    }

    fn flush(&mut self, output: &mut Vec<u8>) -> Result<(), String> {
        while unsafe { allium_x264_delayed_frames(self.0) } > 0 {
            let mut data = std::ptr::null();
            let mut size = 0usize;
            let encoded = unsafe { allium_x264_flush(self.0, &mut data, &mut size) };
            if encoded < 0 || (size != 0 && data.is_null()) {
                return Err("flush direct libx264 encoder failed".into());
            }
            if size != 0 {
                output.extend_from_slice(unsafe { std::slice::from_raw_parts(data, size) });
            }
        }
        Ok(())
    }
}

impl Drop for DirectX264Encoder {
    fn drop(&mut self) {
        unsafe { allium_x264_destroy(self.0) };
    }
}

pub(in crate::animation) fn encode_mp4_into<F>(
    spec: &AnimationEncodeSpec,
    expected: usize,
    config: &H264EncoderConfig,
    telemetry: &mut AnimationEncodeTelemetry,
    mut fill_frame: F,
) -> Result<Vec<u8>, String>
where
    F: FnMut(u32, &mut Vec<u8>) -> Result<FrameCompositeUpdate, String>,
{
    if !spec.width.is_multiple_of(2) || !spec.height.is_multiple_of(2) {
        return Err("H.264 yuv420p requires even animation dimensions".into());
    }
    config.validate()?;
    let mut encoder = DirectX264Encoder::new(spec, config)?;
    let mut access_units = Vec::with_capacity(spec.frame_count as usize);
    let mut sample_bytes = 0usize;
    let mut yuv = Vec::new();
    let mut rgba = Vec::new();
    let mut mb_info = Vec::new();
    for index in 0..spec.frame_count {
        let frame_started = std::time::Instant::now();
        let update = fill_frame(index, &mut rgba)?;
        telemetry.frame_callback_calls += 1;
        telemetry.frame_callback_ns = telemetry
            .frame_callback_ns
            .saturating_add(frame_started.elapsed().as_nanos() as u64);
        if rgba.len() != expected {
            return Err(format!(
                "frame {index} has {} bytes, expected {expected}",
                rgba.len()
            ));
        }
        let yuv_started = std::time::Instant::now();
        match &update {
            FrameCompositeUpdate::Full => {
                rgba_to_yuv420p_into(rgba.as_slice(), spec.width, spec.height, config, &mut yuv)?;
            }
            FrameCompositeUpdate::Dirty(changed_macroblocks) => {
                rgba_to_yuv420p_macroblocks_into(
                    rgba.as_slice(),
                    spec.width,
                    spec.height,
                    changed_macroblocks,
                    config,
                    &mut yuv,
                )?;
            }
            FrameCompositeUpdate::Reused => {}
        }
        telemetry.h264_yuv_ns = telemetry
            .h264_yuv_ns
            .saturating_add(yuv_started.elapsed().as_nanos() as u64);
        let (constant_blocks, total_blocks) =
            fill_x264_mb_info(&update, spec.width, spec.height, &mut mb_info)?;
        telemetry.h264_mbinfo_constant_blocks = telemetry
            .h264_mbinfo_constant_blocks
            .saturating_add(constant_blocks as u64);
        telemetry.h264_mbinfo_total_blocks = telemetry
            .h264_mbinfo_total_blocks
            .saturating_add(total_blocks as u64);
        let encode_started = std::time::Instant::now();
        let mut access_unit = Vec::new();
        encoder.encode(&yuv, &mb_info, index as i64, &mut access_unit)?;
        if access_unit.is_empty() {
            return Err(format!("direct libx264 delayed frame {index}"));
        }
        sample_bytes = sample_bytes.saturating_add(avc_sample_len(&access_unit));
        access_units.push(access_unit);
        telemetry.h264_direct_encode_ns = telemetry
            .h264_direct_encode_ns
            .saturating_add(encode_started.elapsed().as_nanos() as u64);
        // The muxed file holds every sample byte, so stop as soon as the
        // samples alone cross the budget.
        if sample_bytes > spec.output_budget_bytes {
            return Err(output_budget_exceeded(
                sample_bytes,
                spec.output_budget_bytes,
            ));
        }
    }
    let mut delayed = Vec::new();
    encoder.flush(&mut delayed)?;
    if !delayed.is_empty() {
        return Err("direct libx264 produced an unexpected delayed access unit".into());
    }
    drop(encoder);

    let mux_started = std::time::Instant::now();
    let data = mux_avc_mp4(spec.width, spec.height, spec.fps, &access_units)?;
    telemetry.h264_mux_ns = mux_started.elapsed().as_nanos() as u64;
    Ok(data)
}

pub(in crate::animation) fn fill_x264_mb_info(
    update: &FrameCompositeUpdate,
    width: u32,
    height: u32,
    output: &mut Vec<u8>,
) -> Result<(usize, usize), String> {
    const X264_MBINFO_CONSTANT: u8 = 1;
    let mb_width = width.div_ceil(16) as usize;
    let mb_height = height.div_ceil(16) as usize;
    let total = mb_width
        .checked_mul(mb_height)
        .ok_or_else(|| "H.264 macroblock map size overflow".to_string())?;
    output.resize(total, X264_MBINFO_CONSTANT);
    output.fill(X264_MBINFO_CONSTANT);
    match update {
        FrameCompositeUpdate::Full => output.fill(0),
        FrameCompositeUpdate::Reused => {}
        FrameCompositeUpdate::Dirty(changed_macroblocks) => {
            for &index in changed_macroblocks {
                let index = index as usize;
                if index >= total {
                    return Err("H.264 changed macroblock index is out of range".into());
                }
                output[index] = 0;
            }
        }
    }
    Ok((
        output
            .iter()
            .filter(|&&flags| flags & X264_MBINFO_CONSTANT != 0)
            .count(),
        total,
    ))
}

fn mux_avc_mp4(
    width: u32,
    height: u32,
    fps: u32,
    access_units: &[Vec<u8>],
) -> Result<Vec<u8>, String> {
    let mut sps = None;
    let mut pps = None;
    let mut samples = Vec::with_capacity(access_units.len());
    let mut sync_samples = Vec::new();
    for (sample_index, access_unit) in access_units.iter().enumerate() {
        let mut sample = Vec::new();
        let mut sync = false;
        for nal in annex_b_nals(access_unit) {
            if nal.is_empty() {
                continue;
            }
            match nal[0] & 0x1f {
                7 => {
                    sps.get_or_insert_with(|| nal.to_vec());
                }
                8 => {
                    pps.get_or_insert_with(|| nal.to_vec());
                }
                5 => sync = true,
                _ => {}
            };
            if !is_avc_sample_nal(nal) {
                continue;
            }
            mp4_u32(
                &mut sample,
                u32::try_from(nal.len()).map_err(|_| "AVC NAL too large")?,
            );
            sample.extend_from_slice(nal);
        }
        if sample.is_empty() {
            return Err(format!("AVC access unit {sample_index} has no sample NAL"));
        }
        if sync {
            sync_samples.push(sample_index as u32 + 1);
        }
        samples.push(sample);
    }
    let sps = sps.ok_or_else(|| "AVC stream is missing SPS".to_string())?;
    let pps = pps.ok_or_else(|| "AVC stream is missing PPS".to_string())?;
    if sps.len() < 4
        || access_units.is_empty()
        || fps == 0
        || width > u16::MAX as u32
        || height > u16::MAX as u32
    {
        return Err("invalid AVC MP4 track parameters".into());
    }

    let ftyp = mp4_box(*b"ftyp", |payload| {
        payload.extend_from_slice(b"isom");
        mp4_u32(payload, 512);
        payload.extend_from_slice(b"isomiso2avc1mp41");
    })?;
    let sample_sizes = samples
        .iter()
        .map(|sample| u32::try_from(sample.len()).map_err(|_| "AVC sample too large".to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let mdat_payload_bytes = sample_sizes.iter().try_fold(0u32, |total, &size| {
        total
            .checked_add(size)
            .ok_or_else(|| "AVC mdat exceeds 4GiB".to_string())
    })?;
    let mdat_data_offset = u32::try_from(ftyp.len() + 8).map_err(|_| "AVC mdat offset overflow")?;
    let duration = u32::try_from(samples.len()).map_err(|_| "AVC frame count overflow")?;
    let moov = build_avc_moov(
        width as u16,
        height as u16,
        fps,
        duration,
        &sps,
        &pps,
        &sample_sizes,
        &sync_samples,
        mdat_data_offset,
    )?;
    let mut output = Vec::with_capacity(ftyp.len() + 8 + mdat_payload_bytes as usize + moov.len());
    output.extend_from_slice(&ftyp);
    mp4_u32(
        &mut output,
        mdat_payload_bytes
            .checked_add(8)
            .ok_or("AVC mdat size overflow")?,
    );
    output.extend_from_slice(b"mdat");
    for sample in samples {
        output.extend_from_slice(&sample);
    }
    output.extend_from_slice(&moov);
    Ok(output)
}

/// Whether a NAL unit is stored in the MP4 sample. Parameter sets live in the
/// `avcC` box and access unit delimiters are dropped.
fn is_avc_sample_nal(nal: &[u8]) -> bool {
    nal.first()
        .is_some_and(|header| !matches!(header & 0x1f, 7..=9))
}

/// Bytes the access unit occupies as a length-prefixed MP4 sample.
fn avc_sample_len(access_unit: &[u8]) -> usize {
    annex_b_nals(access_unit)
        .into_iter()
        .filter(|nal| is_avc_sample_nal(nal))
        .map(|nal| 4 + nal.len())
        .sum()
}

fn annex_b_nals(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut index = 0usize;
    while index + 3 <= data.len() {
        let prefix = if index + 4 <= data.len() && data[index..index + 4] == [0, 0, 0, 1] {
            4
        } else if data[index..index + 3] == [0, 0, 1] {
            3
        } else {
            index += 1;
            continue;
        };
        starts.push((index, prefix));
        index += prefix;
    }
    starts
        .iter()
        .enumerate()
        .filter_map(|(position, &(start, prefix))| {
            let end = starts
                .get(position + 1)
                .map_or(data.len(), |&(next, _)| next);
            (start + prefix < end).then_some(&data[start + prefix..end])
        })
        .collect()
}

fn mp4_u16(output: &mut Vec<u8>, value: u16) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn mp4_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_be_bytes());
}

fn mp4_box(kind: [u8; 4], fill: impl FnOnce(&mut Vec<u8>)) -> Result<Vec<u8>, String> {
    let mut output = vec![0, 0, 0, 0];
    output.extend_from_slice(&kind);
    fill(&mut output);
    let size = u32::try_from(output.len()).map_err(|_| "MP4 box exceeds 4GiB".to_string())?;
    output[..4].copy_from_slice(&size.to_be_bytes());
    Ok(output)
}

fn mp4_full_box(
    kind: [u8; 4],
    version_and_flags: u32,
    fill: impl FnOnce(&mut Vec<u8>),
) -> Result<Vec<u8>, String> {
    mp4_box(kind, |output| {
        mp4_u32(output, version_and_flags);
        fill(output);
    })
}

#[allow(clippy::too_many_arguments)]
fn build_avc_moov(
    width: u16,
    height: u16,
    timescale: u32,
    duration: u32,
    sps: &[u8],
    pps: &[u8],
    sample_sizes: &[u32],
    sync_samples: &[u32],
    chunk_offset: u32,
) -> Result<Vec<u8>, String> {
    let matrix = [0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];
    let mvhd = mp4_full_box(*b"mvhd", 0, |output| {
        mp4_u32(output, 0);
        mp4_u32(output, 0);
        mp4_u32(output, timescale);
        mp4_u32(output, duration);
        mp4_u32(output, 0x0001_0000);
        mp4_u16(output, 0x0100);
        output.extend_from_slice(&[0; 10]);
        for value in matrix {
            mp4_u32(output, value);
        }
        output.extend_from_slice(&[0; 24]);
        mp4_u32(output, 2);
    })?;
    let tkhd = mp4_full_box(*b"tkhd", 7, |output| {
        mp4_u32(output, 0);
        mp4_u32(output, 0);
        mp4_u32(output, 1);
        mp4_u32(output, 0);
        mp4_u32(output, duration);
        output.extend_from_slice(&[0; 8]);
        mp4_u16(output, 0);
        mp4_u16(output, 0);
        mp4_u16(output, 0);
        mp4_u16(output, 0);
        for value in matrix {
            mp4_u32(output, value);
        }
        mp4_u32(output, u32::from(width) << 16);
        mp4_u32(output, u32::from(height) << 16);
    })?;
    let mdhd = mp4_full_box(*b"mdhd", 0, |output| {
        mp4_u32(output, 0);
        mp4_u32(output, 0);
        mp4_u32(output, timescale);
        mp4_u32(output, duration);
        mp4_u16(output, 0x55c4);
        mp4_u16(output, 0);
    })?;
    let hdlr = mp4_full_box(*b"hdlr", 0, |output| {
        mp4_u32(output, 0);
        output.extend_from_slice(b"vide");
        output.extend_from_slice(&[0; 12]);
        output.extend_from_slice(b"VideoHandler\0");
    })?;
    let vmhd = mp4_full_box(*b"vmhd", 1, |output| output.extend_from_slice(&[0; 8]))?;
    let url = mp4_full_box(*b"url ", 1, |_| {})?;
    let dref = mp4_full_box(*b"dref", 0, |output| {
        mp4_u32(output, 1);
        output.extend_from_slice(&url);
    })?;
    let dinf = mp4_box(*b"dinf", |output| output.extend_from_slice(&dref))?;
    let avcc = mp4_box(*b"avcC", |output| {
        output.extend_from_slice(&[1, sps[1], sps[2], sps[3], 0xff, 0xe1]);
        mp4_u16(output, sps.len() as u16);
        output.extend_from_slice(sps);
        output.push(1);
        mp4_u16(output, pps.len() as u16);
        output.extend_from_slice(pps);
    })?;
    let avc1 = mp4_box(*b"avc1", |output| {
        output.extend_from_slice(&[0; 6]);
        mp4_u16(output, 1);
        output.extend_from_slice(&[0; 16]);
        mp4_u16(output, width);
        mp4_u16(output, height);
        mp4_u32(output, 0x0048_0000);
        mp4_u32(output, 0x0048_0000);
        mp4_u32(output, 0);
        mp4_u16(output, 1);
        let mut compressor = [0u8; 32];
        let name = b"allium libx264";
        compressor[0] = name.len() as u8;
        compressor[1..1 + name.len()].copy_from_slice(name);
        output.extend_from_slice(&compressor);
        mp4_u16(output, 0x0018);
        mp4_u16(output, u16::MAX);
        output.extend_from_slice(&avcc);
    })?;
    let stsd = mp4_full_box(*b"stsd", 0, |output| {
        mp4_u32(output, 1);
        output.extend_from_slice(&avc1);
    })?;
    let stts = mp4_full_box(*b"stts", 0, |output| {
        mp4_u32(output, 1);
        mp4_u32(output, sample_sizes.len() as u32);
        mp4_u32(output, 1);
    })?;
    let stsc = mp4_full_box(*b"stsc", 0, |output| {
        mp4_u32(output, 1);
        mp4_u32(output, 1);
        mp4_u32(output, sample_sizes.len() as u32);
        mp4_u32(output, 1);
    })?;
    let stsz = mp4_full_box(*b"stsz", 0, |output| {
        mp4_u32(output, 0);
        mp4_u32(output, sample_sizes.len() as u32);
        for &size in sample_sizes {
            mp4_u32(output, size);
        }
    })?;
    let stco = mp4_full_box(*b"stco", 0, |output| {
        mp4_u32(output, 1);
        mp4_u32(output, chunk_offset);
    })?;
    let stss = mp4_full_box(*b"stss", 0, |output| {
        mp4_u32(output, sync_samples.len() as u32);
        for &sample in sync_samples {
            mp4_u32(output, sample);
        }
    })?;
    let stbl = mp4_box(*b"stbl", |output| {
        for child in [&stsd, &stts, &stsc, &stsz, &stco, &stss] {
            output.extend_from_slice(child);
        }
    })?;
    let minf = mp4_box(*b"minf", |output| {
        output.extend_from_slice(&vmhd);
        output.extend_from_slice(&dinf);
        output.extend_from_slice(&stbl);
    })?;
    let mdia = mp4_box(*b"mdia", |output| {
        output.extend_from_slice(&mdhd);
        output.extend_from_slice(&hdlr);
        output.extend_from_slice(&minf);
    })?;
    let trak = mp4_box(*b"trak", |output| {
        output.extend_from_slice(&tkhd);
        output.extend_from_slice(&mdia);
    })?;
    mp4_box(*b"moov", |output| {
        output.extend_from_slice(&mvhd);
        output.extend_from_slice(&trak);
    })
}
