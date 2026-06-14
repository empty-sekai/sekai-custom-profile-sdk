//! TMP 富文本段定义。

/// 大小写变换类型。
#[derive(Clone, Debug, PartialEq)]
pub enum CaseTransform {
    None,
    Upper,
    Lower,
}

/// 行首缩进类型。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineIndent {
    Percent(f32),
    Pixels(f32),
}

/// `<size>` 标签尺寸描述。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SizeSpec {
    Absolute(f32),
    Delta(f32),
    Percent(f32),
    Em(f32),
}

/// 缩进与定位描述。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Indent {
    Percent(f32),
    Pixels(f32),
    Em(f32),
}

/// 行内对齐覆盖。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InlineAlign {
    Left,
    Center,
    Right,
}

/// 富文本分段。
#[derive(Clone, Debug)]
pub struct TextSegment {
    pub text: String,
    pub fixed_advance: Option<f32>,
    pub color: Option<(u8, u8, u8)>,
    pub size: Option<SizeSpec>,
    pub scale: Option<f32>,
    pub alpha: Option<f32>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub mark_color: Option<(u8, u8, u8, u8)>,
    pub superscript: bool,
    pub subscript: bool,
    pub case_transform: CaseTransform,
    pub smallcaps: bool,
    pub voffset: Option<f32>,
    pub rotate: Option<f32>,
    pub cspace: Option<f32>,
    pub line_height: Option<f32>,
    pub line_indent: Option<LineIndent>,
    pub indent: Option<Indent>,
    pub position: Option<Indent>,
    pub monospace: Option<Indent>,
    pub duospace: bool,
    pub align: Option<InlineAlign>,
}
