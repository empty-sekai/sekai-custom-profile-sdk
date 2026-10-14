//! The error type of animation encoding and export.

use super::AnimationFormat;
use crate::codec::CodecError;
use crate::render_config::RenderError;
use crate::sdf::tile::SdfTileError;

/// Why an animation could not be configured, encoded, validated or exported.
///
/// The budget, preset and artifact variants render with a stable upper-case
/// code in front of their message (`OUTPUT_BUDGET_EXCEEDED`,
/// `ANIMATION_EXPORT_MEMORY_EXCEEDED`, `ANIMATION_PRESET_UNSUPPORTED`,
/// `ANIMATION_ARTIFACT_INVALID`), so a logged error keeps a searchable prefix.
/// Code that branches on a failure matches the variant instead.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AnimationError {
    /// [`resolve_preset`](crate::animation::resolve_preset) was given a name
    /// that is not a preset.
    #[error("ANIMATION_PRESET_UNSUPPORTED: {0}")]
    UnsupportedPreset(String),
    /// [`plan_page_execution`](crate::animation::plan_page_execution) was
    /// given a static policy other than `passthrough` or `wrap`.
    #[error("unsupported static_policy={0}")]
    UnsupportedStaticPolicy(String),
    /// [`H264YuvConverter::parse`](crate::animation::H264YuvConverter::parse)
    /// was given a name that is not a converter.
    #[error("unsupported H.264 YUV converter: {0}")]
    UnsupportedYuvConverter(String),
    /// The H.264 quantizer is above 51.
    #[error("invalid H.264 QP: {0}")]
    InvalidH264Qp(u8),
    /// AVX-512 conversion was required or validated while the scalar YUV
    /// converter was selected.
    #[error("H.264 AVX-512 was required but the scalar converter was selected")]
    ConflictingYuvConverter,
    /// The H.264 encoder revision is empty or contains a character outside
    /// `[A-Za-z0-9._-]`.
    #[error("H.264 encoder revision must use [A-Za-z0-9._-]")]
    InvalidEncoderRevision,
    /// AVX-512 YUV conversion was required, validated or selected on a CPU
    /// without AVX-512 F and BW.
    #[error("H.264 AVX-512 RGBA-to-YUV420 is unavailable")]
    Avx512Unavailable,
    /// The AVX-512 YUV conversion disagreed with the scalar conversion it is
    /// validated against.
    #[error("H.264 AVX-512 YUV mismatch at byte {offset}: {actual} != {expected}")]
    YuvMismatch {
        offset: usize,
        actual: u8,
        expected: u8,
    },
    /// A width, height, frame count or frame rate is zero.
    #[error("animation dimensions/frame_count/fps must be positive")]
    EmptySpec,
    /// The frame rate is above the 60 Hz clock the scene advances on.
    #[error("animation fps must not exceed the 60Hz core clock, got {0}")]
    FpsAboveCoreClock(u32),
    /// One RGBA frame of these dimensions has a byte length beyond `usize`.
    #[error("animation frame size overflow: {width}x{height}")]
    FrameSizeOverflow { width: u32, height: u32 },
    /// GIF stores frame extents in 16 bits.
    #[error("GIF frames are limited to 65535x65535 pixels, got {width}x{height}")]
    GifFrameTooLarge { width: u32, height: u32 },
    /// H.264 4:2:0 output needs an even width and height.
    #[error("H.264 yuv420p requires even animation dimensions, got {width}x{height}")]
    OddH264Dimensions { width: u32, height: u32 },
    /// A frame source returned a frame whose length is not
    /// `width * height * 4` bytes.
    #[error("frame {index} has {actual} bytes, expected {expected}")]
    FrameLength {
        index: u32,
        actual: usize,
        expected: usize,
    },
    /// A caller-supplied frame source failed with an error of its own.
    #[error("frame source failed: {0}")]
    FrameSource(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The encoded output grew past its output budget.
    #[error("OUTPUT_BUDGET_EXCEEDED: {bytes} > {budget}")]
    OutputBudgetExceeded { bytes: usize, budget: usize },
    /// The budget ladder had no step to attempt.
    #[error("OUTPUT_BUDGET_EXCEEDED: animation budget ladder exhausted")]
    BudgetLadderExhausted,
    /// The retained layer rasters and the frame buffers exceed the export
    /// memory budget.
    #[error("ANIMATION_EXPORT_MEMORY_EXCEEDED: {bytes} > {budget}")]
    ExportMemoryExceeded { bytes: usize, budget: usize },
    /// A format encoder or the MP4 muxer failed; `reason` is the encoder's
    /// own report.
    #[error("{format:?} encoding failed: {reason}")]
    Encoder {
        format: AnimationFormat,
        reason: String,
    },
    /// Compressing the pixel data of an APNG frame failed.
    #[error("APNG frame {frame} compression failed: {source}")]
    ApngCompression {
        frame: u32,
        #[source]
        source: std::io::Error,
    },
    /// An encoded artifact is not a well-formed container of its format.
    #[error("ANIMATION_ARTIFACT_INVALID: {format:?} {reason}")]
    InvalidArtifact {
        format: AnimationFormat,
        reason: &'static str,
    },
    /// A stored artifact's path extension, content type or byte length
    /// disagrees with its format and record.
    #[error("ANIMATION_ARTIFACT_INVALID: path/mime/size mismatch")]
    ArtifactMismatch,
    /// The page given to [`wrap_static_png`](crate::animation::wrap_static_png)
    /// is not a decodable PNG.
    #[error("decode static PNG for animation wrap failed: {0}")]
    StaticPng(#[source] CodecError),
    /// Building or preflighting the card's scene failed.
    #[error(transparent)]
    Scene(#[from] sekai_profile_core::CoreError),
    /// Resolving the render config, resolving the card or rendering one of
    /// its layers failed.
    #[error(transparent)]
    Render(#[from] RenderError),
    /// Planning or executing the SDF tiles of a layer failed.
    #[error(transparent)]
    SdfTile(#[from] SdfTileError),
    /// A size or buffer invariant of the export pipeline did not hold.
    #[error("{0}")]
    Internal(&'static str),
}

impl AnimationError {
    /// An [`AnimationError::Encoder`] for `format`.
    pub(crate) fn encoder(format: AnimationFormat, reason: impl Into<String>) -> Self {
        Self::Encoder {
            format,
            reason: reason.into(),
        }
    }

    /// Whether the output outgrew its budget, which a smaller budget step may
    /// avoid.
    pub(crate) fn is_output_budget_exceeded(&self) -> bool {
        matches!(
            self,
            Self::OutputBudgetExceeded { .. } | Self::BudgetLadderExhausted
        )
    }
}
