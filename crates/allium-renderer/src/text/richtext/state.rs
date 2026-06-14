//! 富文本解析状态机。

use super::types::{CaseTransform, Indent, InlineAlign, LineIndent, SizeSpec, TextSegment};

pub(crate) type ColorStackEntry = (Option<(u8, u8, u8)>, Option<f32>);

pub(crate) struct RichTextParseState {
    pub(crate) segs: Vec<TextSegment>,
    pub(crate) color_stack: Vec<ColorStackEntry>,
    pub(crate) current_color: Option<(u8, u8, u8)>,
    pub(crate) size_stack: Vec<SizeSpec>,
    pub(crate) scale_stack: Vec<f32>,
    pub(crate) alpha_override: Option<f32>,
    pub(crate) bold_depth: i32,
    pub(crate) italic_depth: i32,
    pub(crate) underline_depth: i32,
    pub(crate) strikethrough_depth: i32,
    pub(crate) subscript_depth: i32,
    pub(crate) superscript_depth: i32,
    pub(crate) mark_stack: Vec<(u8, u8, u8, u8)>,
    pub(crate) case_stack: Vec<CaseTransform>,
    pub(crate) smallcaps_depth: i32,
    pub(crate) noparse_depth: i32,
    pub(crate) voffset_stack: Vec<f32>,
    pub(crate) rotate_stack: Vec<f32>,
    pub(crate) cspace_override: Option<f32>,
    pub(crate) line_height_override: Option<f32>,
    pub(crate) line_indent_override: Option<LineIndent>,
    pub(crate) indent_stack: Vec<Indent>,
    pub(crate) position_stack: Vec<Indent>,
    pub(crate) monospace_stack: Vec<Indent>,
    pub(crate) duospace_stack: Vec<bool>,
    pub(crate) align_stack: Vec<InlineAlign>,
}

impl RichTextParseState {
    pub(crate) fn new() -> Self {
        Self {
            segs: Vec::new(),
            color_stack: vec![(None, None)],
            current_color: None,
            size_stack: Vec::new(),
            scale_stack: Vec::new(),
            alpha_override: None,
            bold_depth: 0,
            italic_depth: 0,
            underline_depth: 0,
            strikethrough_depth: 0,
            subscript_depth: 0,
            superscript_depth: 0,
            mark_stack: Vec::new(),
            case_stack: Vec::new(),
            smallcaps_depth: 0,
            noparse_depth: 0,
            voffset_stack: Vec::new(),
            rotate_stack: Vec::new(),
            cspace_override: None,
            line_height_override: None,
            line_indent_override: None,
            indent_stack: Vec::new(),
            position_stack: Vec::new(),
            monospace_stack: Vec::new(),
            duospace_stack: Vec::new(),
            align_stack: Vec::new(),
        }
    }

    pub(crate) fn into_segments(self) -> Vec<TextSegment> {
        self.segs
    }
    pub(crate) fn noparse_depth(&self) -> i32 {
        self.noparse_depth
    }

    pub(crate) fn append_char(&mut self, ch: char) {
        let seg = self.build_segment(&ch.to_string(), None);
        if self.can_merge_with_last(&seg) {
            if let Some(last) = self.segs.last_mut() {
                last.text.push(ch);
            }
        } else {
            self.segs.push(seg);
        }
    }

    pub(crate) fn push_text(&mut self, text: &str) {
        self.segs.push(self.build_segment(text, None));
    }
    pub(crate) fn push_fixed_advance(&mut self, fixed_advance: f32) {
        self.segs.push(self.build_segment("", Some(fixed_advance)));
    }

    pub(crate) fn build_segment(&self, text: &str, fixed_advance: Option<f32>) -> TextSegment {
        TextSegment {
            text: text.to_string(),
            fixed_advance,
            color: self.current_color,
            size: self.size_stack.last().copied(),
            scale: self.scale_stack.last().copied(),
            alpha: self.alpha_override,
            bold: self.bold_depth > 0,
            italic: self.italic_depth > 0,
            underline: self.underline_depth > 0,
            strikethrough: self.strikethrough_depth > 0,
            mark_color: self.mark_stack.last().copied(),
            superscript: self.superscript_depth > 0,
            subscript: self.subscript_depth > 0,
            case_transform: self
                .case_stack
                .last()
                .cloned()
                .unwrap_or(CaseTransform::None),
            smallcaps: self.smallcaps_depth > 0,
            voffset: self.voffset_stack.last().copied(),
            rotate: self.rotate_stack.last().copied(),
            cspace: self.cspace_override,
            line_height: self.line_height_override,
            line_indent: self.line_indent_override,
            indent: self.indent_stack.last().copied(),
            position: self.position_stack.last().copied(),
            monospace: self.monospace_stack.last().copied(),
            duospace: self.duospace_stack.last().copied().unwrap_or(false),
            align: self.align_stack.last().copied(),
        }
    }

    fn can_merge_with_last(&self, seg: &TextSegment) -> bool {
        self.segs.last().is_some_and(|last| {
            last.color == seg.color
                && last.fixed_advance == seg.fixed_advance
                && last.size == seg.size
                && last.scale == seg.scale
                && last.alpha == seg.alpha
                && last.bold == seg.bold
                && last.italic == seg.italic
                && last.underline == seg.underline
                && last.strikethrough == seg.strikethrough
                && last.mark_color == seg.mark_color
                && last.superscript == seg.superscript
                && last.subscript == seg.subscript
                && last.case_transform == seg.case_transform
                && last.smallcaps == seg.smallcaps
                && last.voffset == seg.voffset
                && last.rotate == seg.rotate
                && last.cspace == seg.cspace
                && last.line_height == seg.line_height
                && last.line_indent == seg.line_indent
                && last.indent == seg.indent
                && last.position == seg.position
                && last.monospace == seg.monospace
                && last.duospace == seg.duospace
                && last.align == seg.align
        })
    }
}
