//! TextMesh Pro rich-text markup.
//!
//! [`parse_segments`] turns profile text into styled runs for the layout
//! engines, and [`visible_scalars`] lists the characters the markup leaves in
//! the text together with the source bytes each one came from. Both run the
//! same grammar: a `<...>` sequence that TextMesh Pro would not accept as a tag
//! stays in the text as literal characters.
//!
//! Tag names match in all-lowercase or all-uppercase form only, and named
//! colours and alignment values in lowercase only. Lengths carry the unit they
//! were written in: a bare number or `px` is in layout units, `em` is a
//! multiple of the font size in effect at the tag, and `%` is accepted only by
//! the tags that define it. `<br>`, `<nbsp>` and `<zwsp>` are replaced by their
//! characters before any tag is read, in any letter case and even inside
//! `<noparse>`.

use std::ops::Range;

use super::face::{TmpFaceInfo, PROFILE_FACE};

/// Longest tag body read before a `<` is taken literally.
const MAX_TAG_CHARS: usize = 128;
/// Attribute slots one tag may fill.
const MAX_TAG_ATTRIBUTES: usize = 8;
/// The value TextMesh Pro's number reader reports for a missing or
/// out-of-range number.
const INVALID_NUMBER: f32 = -32768.0;

/// A horizontal distance in layout units, or a share of the text container
/// width that only the layout can resolve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HorizontalOffset {
    Units(f32),
    Percent(f32),
}

/// A caret movement that is not tied to a character.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CaretCommand {
    /// Moves the caret by a distance in layout units: `<space>`, and the
    /// character spacing `</cspace>` takes back from the last character.
    Advance(f32),
    /// Places the caret at an offset from the start of the line: `<pos>`.
    MoveTo(HorizontalOffset),
}

/// Letter case applied to the characters of a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaseTransform {
    None,
    Upper,
    Lower,
    /// Lowercase letters are drawn as uppercase at [`SMALL_CAPS_SCALE`].
    SmallCaps,
}

/// Size of a `<smallcaps>` letter relative to the run's font size.
pub const SMALL_CAPS_SCALE: f32 = 0.8;

/// `<align>` override for the lines a run starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineAlign {
    Left,
    Center,
    Right,
    Justified,
    Flush,
}

/// One run of text sharing a single style.
///
/// Lengths are in layout units, the units of the element's font size scaled
/// by [`TmpFaceInfo::scale`] where TextMesh Pro applies it.
#[derive(Clone, Debug, PartialEq)]
pub struct TextSegment {
    /// The visible characters of the run; `'\n'` breaks the line. Empty for a
    /// caret command.
    pub text: String,
    pub caret: Option<CaretCommand>,
    /// Face colour; `None` keeps the element colour.
    pub color: Option<[u8; 3]>,
    /// Upper bound on the element alpha; 255 leaves it unchanged.
    pub alpha: u8,
    /// Font size in effect, before sub/superscript scaling.
    pub font_size: f32,
    /// Sub/superscript size multiplier; 1 outside those tags.
    pub font_scale: f32,
    /// Baseline shift, positive upward.
    pub baseline_offset: f32,
    /// `<scale>` horizontal factor. `<scale>` and `<rotate>` share one
    /// transform, so at most one of the two is set.
    pub scale: Option<f32>,
    /// `<rotate>` angle in degrees, counter-clockwise.
    pub rotate: Option<f32>,
    pub bold: bool,
    /// Italic slant in hundredths of the glyph height; `None` is upright.
    pub italic: Option<i32>,
    pub underline: bool,
    pub strikethrough: bool,
    /// `<mark>` highlight colour, RGBA.
    pub mark: Option<[u8; 4]>,
    pub case: CaseTransform,
    /// Extra advance after every character (`<cspace>`).
    pub character_spacing: f32,
    /// Fixed advance per character (`<mspace>`).
    pub monospace: Option<f32>,
    /// Fixed distance between baselines (`<line-height>`).
    pub line_height: Option<f32>,
    /// Caret position at the start of every line (`<indent>`).
    pub indent: Option<HorizontalOffset>,
    /// Extra indent of the first line after the tag (`<line-indent>`).
    pub line_indent: Option<HorizontalOffset>,
    pub align: Option<InlineAlign>,
    /// Inside `<nobr>`: white space and CJK characters offer no line break.
    pub no_break: bool,
}

impl TextSegment {
    /// The size glyphs are drawn and measured at: the font size with the
    /// sub/superscript multiplier applied.
    pub fn render_size(&self) -> f32 {
        self.font_size * self.font_scale
    }

    /// The character drawn for `ch` in this run and its size factor.
    ///
    /// Case changes map one character to one character; a letter without a
    /// single-character counterpart is left unchanged.
    pub fn transform_char(&self, ch: char) -> (char, f32) {
        match self.case {
            CaseTransform::Upper if ch.is_lowercase() => (simple_upper(ch), 1.0),
            CaseTransform::Lower if ch.is_uppercase() => (simple_lower(ch), 1.0),
            CaseTransform::SmallCaps if ch.is_lowercase() => (simple_upper(ch), SMALL_CAPS_SCALE),
            _ => (ch, 1.0),
        }
    }
}

fn simple_upper(ch: char) -> char {
    single(ch.to_uppercase()).unwrap_or(ch)
}

fn simple_lower(ch: char) -> char {
    single(ch.to_lowercase()).unwrap_or(ch)
}

fn single(mut chars: impl Iterator<Item = char>) -> Option<char> {
    let first = chars.next()?;
    chars.next().is_none().then_some(first)
}

/// A character the markup leaves in the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisibleScalar {
    pub ch: char,
    /// Byte range of the source that produced it: the character itself, or
    /// the whole `<br>`, `<nbsp>` or `<zwsp>` tag.
    pub source: Range<usize>,
    /// Inside `<nobr>`.
    pub no_break: bool,
}

/// Parses profile text into styled runs for an element whose font size is
/// `font_size`, using the profile font metrics.
pub fn parse_segments(raw: &str, font_size: f32) -> Vec<TextSegment> {
    parse_segments_with_face(raw, font_size, &PROFILE_FACE)
}

/// [`parse_segments`] for an explicit face.
pub fn parse_segments_with_face(raw: &str, font_size: f32, face: &TmpFaceInfo) -> Vec<TextSegment> {
    let mut segments: Vec<TextSegment> = Vec::new();
    run_markup(raw, font_size, face, |event, state| match event {
        MarkupEvent::Scalar { ch, .. } => {
            let mut segment = state.segment(String::new(), None);
            if let Some(last) = segments.last_mut().filter(|last| last.caret.is_none()) {
                if same_style(last, &segment) {
                    last.text.push(ch);
                    return;
                }
            }
            segment.text.push(ch);
            segments.push(segment);
        }
        MarkupEvent::Caret(command) => segments.push(state.segment(String::new(), Some(command))),
    });
    segments
}

/// The characters the markup leaves in the text, in order.
pub fn visible_scalars(raw: &str) -> Vec<VisibleScalar> {
    let mut scalars = Vec::new();
    run_markup(raw, 0.0, &PROFILE_FACE, |event, state| {
        if let MarkupEvent::Scalar { ch, source } = event {
            scalars.push(VisibleScalar {
                ch,
                source,
                no_break: state.no_break,
            });
        }
    });
    scalars
}

/// Whether `last` and the empty-text `next` carry the same style.
fn same_style(last: &mut TextSegment, next: &TextSegment) -> bool {
    let text = std::mem::take(&mut last.text);
    let same = *last == *next;
    last.text = text;
    same
}

enum MarkupEvent {
    Scalar { ch: char, source: Range<usize> },
    Caret(CaretCommand),
}

fn run_markup(
    raw: &str,
    font_size: f32,
    face: &TmpFaceInfo,
    mut sink: impl FnMut(MarkupEvent, &MarkupState),
) {
    let scalars = preprocess(raw);
    let chars: Vec<char> = scalars.iter().map(|(ch, _)| *ch).collect();
    let mut state = MarkupState::new(font_size, *face);
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '<' {
            if let Some((tag, end)) = scan_tag(&chars, index + 1) {
                match state.apply(&tag) {
                    TagEffect::Rejected => {}
                    TagEffect::Applied => {
                        index = end + 1;
                        continue;
                    }
                    TagEffect::Caret(command) => {
                        sink(MarkupEvent::Caret(command), &state);
                        index = end + 1;
                        continue;
                    }
                }
            }
        }
        state.characters_emitted = true;
        sink(
            MarkupEvent::Scalar {
                ch: chars[index],
                source: scalars[index].1.clone(),
            },
            &state,
        );
        index += 1;
    }
}

/// Replaces the tags TextMesh Pro resolves before reading any markup. The
/// name is matched case-insensitively up to the first `>`, `=` or space, and
/// the replacement always consumes the tag's nominal length.
fn preprocess(raw: &str) -> Vec<(char, Range<usize>)> {
    let chars: Vec<(usize, char)> = raw.char_indices().collect();
    let offset_at = |index: usize| chars.get(index).map_or(raw.len(), |(offset, _)| *offset);
    let mut out = Vec::with_capacity(chars.len());
    let mut index = 0;
    while index < chars.len() {
        let (offset, ch) = chars[index];
        if ch == '<' {
            let name: String = chars[index + 1..]
                .iter()
                .map(|(_, c)| *c)
                .take(16)
                .take_while(|c| !matches!(c, '>' | '=' | ' '))
                .map(|c| c.to_ascii_uppercase())
                .collect();
            let replacement = match name.as_str() {
                "BR" => Some((Some('\n'), 4)),
                "NBSP" => Some((Some('\u{00A0}'), 6)),
                "ZWSP" => Some((Some('\u{200B}'), 6)),
                // No style sheet is available, so closing a style inserts
                // nothing and only the tag itself is consumed.
                "/STYLE" => Some((None, 8)),
                _ => None,
            };
            if let Some((replacement, length)) = replacement {
                let end = (index + length).min(chars.len());
                if let Some(replacement) = replacement {
                    out.push((replacement, offset..offset_at(end)));
                }
                index = end;
                continue;
            }
        }
        out.push((ch, offset..offset + ch.len_utf8()));
        index += 1;
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ValueType {
    None,
    Number,
    Color,
    Text,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unit {
    Pixels,
    FontUnits,
    Percentage,
}

#[derive(Clone, Debug)]
struct Attribute {
    name: String,
    value_type: ValueType,
    value_start: usize,
    value_length: usize,
}

impl Attribute {
    fn empty() -> Self {
        Self {
            name: String::new(),
            value_type: ValueType::None,
            value_start: 0,
            value_length: 0,
        }
    }
}

struct ScannedTag {
    chars: Vec<char>,
    attributes: Vec<Attribute>,
    /// Unit of the most recent value, which is what the tags read.
    unit: Unit,
}

impl ScannedTag {
    fn name(&self) -> &str {
        &self.attributes[0].name
    }

    fn value(&self, index: usize) -> String {
        let attribute = &self.attributes[index];
        let end = (attribute.value_start + attribute.value_length).min(self.chars.len());
        self.chars[attribute.value_start.min(end)..end]
            .iter()
            .collect()
    }

    fn number(&self, index: usize) -> Option<f32> {
        let attribute = &self.attributes[index];
        convert_to_float(&self.chars, attribute.value_start, attribute.value_length)
    }

    fn char_at(&self, index: usize) -> char {
        self.chars.get(index).copied().unwrap_or('\0')
    }
}

/// Reads the tag body starting after a `<`. Returns the tag and the index of
/// its closing `>`, or `None` when the characters cannot form a tag.
fn scan_tag(chars: &[char], start: usize) -> Option<(ScannedTag, usize)> {
    let mut tag: Vec<char> = Vec::new();
    let mut attributes = vec![Attribute::empty()];
    let mut current = 0usize;
    let mut flag = 0u8;
    let mut value_type = ValueType::None;
    let mut unit = Unit::Pixels;
    let mut name_set = false;
    let mut index = start;
    while index < chars.len()
        && chars[index] != '\0'
        && tag.len() < MAX_TAG_CHARS
        && chars[index] != '<'
    {
        let ch = chars[index];
        if ch == '>' {
            return Some((
                ScannedTag {
                    chars: tag,
                    attributes,
                    unit,
                },
                index,
            ));
        }
        tag.push(ch);
        let count = tag.len();
        let mut next_attribute = false;
        if flag == 1 {
            let attribute = &mut attributes[current];
            match value_type {
                ValueType::None => {
                    unit = Unit::Pixels;
                    if matches!(ch, '+' | '-' | '.') || ch.is_ascii_digit() {
                        value_type = ValueType::Number;
                        attribute.value_start = count - 1;
                        attribute.value_length += 1;
                    } else if ch == '#' {
                        value_type = ValueType::Color;
                        attribute.value_start = count - 1;
                        attribute.value_length += 1;
                    } else if ch == '"' {
                        value_type = ValueType::Text;
                        attribute.value_start = count;
                    } else {
                        value_type = ValueType::Text;
                        attribute.value_start = count - 1;
                        attribute.value_length += 1;
                    }
                    attribute.value_type = value_type;
                }
                ValueType::Number => {
                    if matches!(ch, 'p' | 'e' | '%' | ' ') {
                        flag = 2;
                        value_type = ValueType::None;
                        unit = match ch {
                            'e' => Unit::FontUnits,
                            '%' => Unit::Percentage,
                            _ => Unit::Pixels,
                        };
                        next_attribute = true;
                    } else {
                        attribute.value_length += 1;
                    }
                }
                ValueType::Color => {
                    if ch != ' ' {
                        attribute.value_length += 1;
                    } else {
                        flag = 2;
                        value_type = ValueType::None;
                        unit = Unit::Pixels;
                        next_attribute = true;
                    }
                }
                ValueType::Text => {
                    if ch != '"' {
                        attribute.value_length += 1;
                    } else {
                        flag = 2;
                        value_type = ValueType::None;
                        unit = Unit::Pixels;
                        next_attribute = true;
                    }
                }
            }
        }
        if ch == '=' {
            flag = 1;
        }
        if flag == 0 && ch == ' ' {
            if name_set {
                return None;
            }
            name_set = true;
            flag = 2;
            value_type = ValueType::None;
            unit = Unit::Pixels;
            next_attribute = true;
        }
        if next_attribute {
            current += 1;
            if current >= MAX_TAG_ATTRIBUTES {
                return None;
            }
            attributes.push(Attribute::empty());
        }
        if flag == 0 {
            attributes[current].name.push(ch);
        }
        if flag == 2 && ch == ' ' {
            flag = 0;
        }
        index += 1;
    }
    None
}

/// TextMesh Pro's number reader: digits and `.` accumulate, a `,` ends the
/// value, and every other character is skipped. A missing value or one above
/// 32767 is rejected.
fn convert_to_float(chars: &[char], start: usize, length: usize) -> Option<f32> {
    if start == 0 {
        return None;
    }
    let end = (start + length).min(chars.len());
    let mut index = start;
    let mut sign = 1.0f32;
    match chars.get(index) {
        Some('+') => index += 1,
        Some('-') => {
            sign = -1.0;
            index += 1;
        }
        _ => {}
    }
    let mut value = 0.0f32;
    let mut integer = true;
    let mut decimal = 0.0f32;
    while index < end {
        let ch = chars[index];
        index += 1;
        if ch == '.' {
            integer = false;
            decimal = 0.1;
        } else if let Some(digit) = ch.to_digit(10) {
            let digit = digit as f32;
            if integer {
                value = value * 10.0 + digit * sign;
            } else {
                value += digit * decimal * sign;
                decimal *= 0.1;
            }
        } else if ch == ',' {
            break;
        }
    }
    (value <= 32767.0 && value != INVALID_NUMBER).then_some(value)
}

fn hex_nibble(ch: char) -> u8 {
    match ch {
        '0'..='9' => ch as u8 - b'0',
        'a'..='f' => ch as u8 - b'a' + 10,
        'A'..='F' => ch as u8 - b'A' + 10,
        _ => 15,
    }
}

fn hex_pair(high: char, low: char) -> u8 {
    hex_nibble(high) * 16 + hex_nibble(low)
}

/// RGBA from the characters of a short or long hex colour. `digits` holds the
/// characters after the `#`.
fn hex_color(digits: &[char]) -> Option<[u8; 4]> {
    let short = |index: usize| hex_pair(digits[index], digits[index]);
    match digits.len() {
        3 => Some([short(0), short(1), short(2), 255]),
        4 => Some([short(0), short(1), short(2), short(3)]),
        6 => Some([
            hex_pair(digits[0], digits[1]),
            hex_pair(digits[2], digits[3]),
            hex_pair(digits[4], digits[5]),
            255,
        ]),
        8 => Some([
            hex_pair(digits[0], digits[1]),
            hex_pair(digits[2], digits[3]),
            hex_pair(digits[4], digits[5]),
            hex_pair(digits[6], digits[7]),
        ]),
        _ => None,
    }
}

/// An attribute colour: `#RRGGBB` or `#RRGGBBAA`, anything else is white.
fn attribute_color(tag: &ScannedTag, index: usize) -> [u8; 4] {
    let attribute = &tag.attributes[index];
    let end = (attribute.value_start + attribute.value_length).min(tag.chars.len());
    let value = &tag.chars[attribute.value_start.min(end)..end];
    match value.len() {
        7 | 9 => hex_color(&value[1..]).unwrap_or([255; 4]),
        _ => [255; 4],
    }
}

fn named_color(name: &str) -> Option<[u8; 3]> {
    Some(match name {
        "red" => [255, 0, 0],
        "lightblue" => [173, 216, 230],
        "blue" => [0, 0, 255],
        "grey" => [128, 128, 128],
        "black" => [0, 0, 0],
        "green" => [0, 255, 0],
        "white" => [255, 255, 255],
        "orange" => [255, 128, 0],
        "purple" => [160, 32, 240],
        "yellow" => [255, 235, 4],
        _ => return None,
    })
}

/// Whether a tag or attribute name is `lower` written in all-lowercase or
/// all-uppercase letters.
fn is_name(name: &str, lower: &str) -> bool {
    name == lower
        || (name.len() == lower.len()
            && name
                .bytes()
                .zip(lower.bytes())
                .all(|(a, b)| a == b.to_ascii_uppercase()))
}

enum TagEffect {
    Rejected,
    Applied,
    Caret(CaretCommand),
}

#[derive(Clone, Copy, PartialEq)]
struct HtmlColor {
    rgb: Option<[u8; 3]>,
    alpha: u8,
}

/// A stack that always keeps its bottom item, like the ones TextMesh Pro
/// restores closing tags from.
struct RestoreStack<T: Copy> {
    items: Vec<T>,
}

impl<T: Copy> RestoreStack<T> {
    fn new(default: T) -> Self {
        Self {
            items: vec![default],
        }
    }

    fn add(&mut self, item: T) {
        self.items.push(item);
    }

    fn remove(&mut self) -> T {
        if self.items.len() > 1 {
            self.items.pop();
        }
        self.items[self.items.len() - 1]
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Effect {
    None,
    Scale(f32),
    Rotate(f32),
}

#[derive(Default)]
struct StyleCounts {
    bold: u32,
    italic: u32,
    underline: u32,
    strikethrough: u32,
    highlight: u32,
    subscript: u32,
    superscript: u32,
    lowercase: u32,
    uppercase: u32,
    smallcaps: u32,
}

fn style_add(count: &mut u32) {
    *count = count.saturating_add(1);
}

fn style_remove(count: &mut u32) {
    *count = count.saturating_sub(1);
}

struct MarkupState {
    face: TmpFaceInfo,
    base_font_size: f32,
    font_size: f32,
    size_stack: RestoreStack<f32>,
    font_scale: f32,
    baseline_offset: f32,
    baseline_stack: Vec<f32>,
    styles: StyleCounts,
    italic_angle: i32,
    italic_angle_stack: RestoreStack<i32>,
    color: HtmlColor,
    color_stack: RestoreStack<HtmlColor>,
    highlight_stack: RestoreStack<Option<[u8; 4]>>,
    character_spacing: f32,
    monospace: f32,
    line_height: Option<f32>,
    indent: Option<HorizontalOffset>,
    indent_stack: RestoreStack<Option<HorizontalOffset>>,
    line_indent: Option<HorizontalOffset>,
    effect: Effect,
    align: Option<InlineAlign>,
    align_stack: RestoreStack<Option<InlineAlign>>,
    no_break: bool,
    no_parse: bool,
    characters_emitted: bool,
}

impl MarkupState {
    fn new(font_size: f32, face: TmpFaceInfo) -> Self {
        let color = HtmlColor {
            rgb: None,
            alpha: 255,
        };
        Self {
            face,
            base_font_size: font_size,
            font_size,
            size_stack: RestoreStack::new(font_size),
            font_scale: 1.0,
            baseline_offset: 0.0,
            baseline_stack: Vec::new(),
            styles: StyleCounts::default(),
            italic_angle: face.italic_style,
            italic_angle_stack: RestoreStack::new(face.italic_style),
            color,
            color_stack: RestoreStack::new(color),
            highlight_stack: RestoreStack::new(None),
            character_spacing: 0.0,
            monospace: 0.0,
            line_height: None,
            indent: None,
            indent_stack: RestoreStack::new(None),
            line_indent: None,
            effect: Effect::None,
            align: None,
            align_stack: RestoreStack::new(None),
            no_break: false,
            no_parse: false,
            characters_emitted: false,
        }
    }

    fn segment(&self, text: String, caret: Option<CaretCommand>) -> TextSegment {
        let styles = &self.styles;
        let case = if styles.uppercase > 0 {
            CaseTransform::Upper
        } else if styles.lowercase > 0 {
            CaseTransform::Lower
        } else if styles.smallcaps > 0 {
            CaseTransform::SmallCaps
        } else {
            CaseTransform::None
        };
        let (scale, rotate) = match self.effect {
            Effect::None => (None, None),
            Effect::Scale(value) => (Some(value), None),
            Effect::Rotate(value) => (None, Some(value)),
        };
        TextSegment {
            text,
            caret,
            color: self.color.rgb,
            alpha: self.color.alpha,
            font_size: self.font_size,
            font_scale: self.font_scale,
            baseline_offset: self.baseline_offset,
            scale,
            rotate,
            bold: styles.bold > 0,
            italic: (styles.italic > 0).then_some(self.italic_angle),
            underline: styles.underline > 0,
            strikethrough: styles.strikethrough > 0,
            mark: if styles.highlight > 0 {
                self.highlight_stack.items.last().copied().flatten()
            } else {
                None
            },
            case,
            character_spacing: self.character_spacing,
            monospace: (self.monospace != 0.0).then_some(self.monospace),
            line_height: self.line_height,
            indent: self.indent,
            line_indent: self.line_indent,
            align: self.align,
            no_break: self.no_break,
        }
    }

    fn set_color(&mut self, rgba: [u8; 4]) {
        self.color = HtmlColor {
            rgb: Some([rgba[0], rgba[1], rgba[2]]),
            alpha: rgba[3],
        };
        self.color_stack.add(self.color);
    }

    /// A length in layout units for the pixel and font-unit forms, `None`
    /// for a percentage.
    fn length(&self, tag: &ScannedTag, value: f32) -> Option<f32> {
        match tag.unit {
            Unit::Pixels => Some(value),
            Unit::FontUnits => Some(value * self.font_size),
            Unit::Percentage => None,
        }
    }

    fn offset(&self, tag: &ScannedTag, value: f32) -> HorizontalOffset {
        match tag.unit {
            Unit::Pixels => HorizontalOffset::Units(value),
            Unit::FontUnits => HorizontalOffset::Units(value * self.font_size),
            Unit::Percentage => HorizontalOffset::Percent(value),
        }
    }

    fn apply(&mut self, tag: &ScannedTag) -> TagEffect {
        let name = tag.name();
        if self.no_parse && !is_name(name, "/noparse") {
            return TagEffect::Rejected;
        }
        if is_name(name, "/noparse") {
            self.no_parse = false;
            return TagEffect::Applied;
        }
        if tag.chars.first() == Some(&'#') && matches!(tag.chars.len(), 4 | 5 | 7 | 9) {
            if let Some(rgba) = hex_color(&tag.chars[1..]) {
                self.set_color(rgba);
                return TagEffect::Applied;
            }
        }
        if self.apply_named(tag) {
            TagEffect::Applied
        } else {
            match self.apply_valued(tag) {
                Some(effect) => effect,
                None => TagEffect::Rejected,
            }
        }
    }

    /// Tags whose effect does not depend on a value.
    fn apply_named(&mut self, tag: &ScannedTag) -> bool {
        let name = tag.name();
        let styles = &mut self.styles;
        let is = |lower: &str| is_name(name, lower);
        if is("b") {
            style_add(&mut styles.bold);
        } else if is("/b") {
            style_remove(&mut styles.bold);
        } else if is("/i") {
            self.italic_angle = self.italic_angle_stack.remove();
            style_remove(&mut styles.italic);
        } else if is("s") {
            style_add(&mut styles.strikethrough);
        } else if is("/s") {
            style_remove(&mut styles.strikethrough);
        } else if is("u") {
            style_add(&mut styles.underline);
        } else if is("/u") {
            style_remove(&mut styles.underline);
        } else if is("/mark") {
            self.highlight_stack.remove();
            style_remove(&mut styles.highlight);
        } else if is("sub") || is("sup") {
            let superscript = is("sup");
            let (size, offset) = if superscript {
                (self.face.superscript_size, self.face.superscript_offset)
            } else {
                (self.face.subscript_size, self.face.subscript_offset)
            };
            self.font_scale *= if size > 0.0 { size } else { 1.0 };
            self.baseline_stack.push(self.baseline_offset);
            self.baseline_offset += offset * self.face.font_scale(self.font_size) * self.font_scale;
            if superscript {
                style_add(&mut self.styles.superscript);
            } else {
                style_add(&mut self.styles.subscript);
            }
        } else if is("/sub") || is("/sup") {
            let superscript = is("/sup");
            let (count, size) = if superscript {
                (&mut self.styles.superscript, self.face.superscript_size)
            } else {
                (&mut self.styles.subscript, self.face.subscript_size)
            };
            if *count > 0 {
                if self.font_scale < 1.0 {
                    self.baseline_offset = self.baseline_stack.pop().unwrap_or(0.0);
                    self.font_scale /= if size > 0.0 { size } else { 1.0 };
                }
                style_remove(count);
            }
        } else if is("/font-weight") || is("/pos") || is("/font") || is("/material") {
            // Weight variants, fonts and materials other than the default
            // are not available; closing them changes nothing drawn.
        } else if is("/voffset") {
            self.baseline_offset = 0.0;
        } else if is("page")
            || is("/a")
            || is("/link")
            || is("/gradient")
            || is("action")
            || is("/action")
            || is("/width")
            || is("/margin")
            || is("/table")
            || is("tr")
            || is("/tr")
            || is("th")
            || is("/th")
        {
            // Accepted, but nothing a single text box draws depends on them.
        } else if is("nobr") {
            self.no_break = true;
        } else if is("/nobr") {
            self.no_break = false;
        } else if is("/size") {
            self.font_size = self.size_stack.remove();
        } else if is("/align") {
            self.align = self.align_stack.remove();
        } else if is("/cspace") {
            // Handled with its caret retraction in `apply_valued`.
            return false;
        } else if is("/mspace") {
            self.monospace = 0.0;
        } else if is("/color") {
            self.color = self.color_stack.remove();
        } else if is("/indent") {
            self.indent = self.indent_stack.remove();
        } else if is("/line-indent") {
            self.line_indent = None;
        } else if is("lowercase") {
            style_add(&mut styles.lowercase);
        } else if is("/lowercase") {
            style_remove(&mut styles.lowercase);
        } else if is("allcaps") || is("uppercase") {
            style_add(&mut styles.uppercase);
        } else if is("/allcaps") || is("/uppercase") {
            style_remove(&mut styles.uppercase);
        } else if is("smallcaps") {
            style_add(&mut styles.smallcaps);
        } else if is("/smallcaps") {
            style_remove(&mut styles.smallcaps);
        } else if is("/line-height") {
            self.line_height = None;
        } else if is("noparse") {
            self.no_parse = true;
        } else if is("/scale") || is("/rotate") {
            self.effect = Effect::None;
        } else {
            return false;
        }
        true
    }

    /// Tags that read a value or may reject the tag. `None` rejects it.
    fn apply_valued(&mut self, tag: &ScannedTag) -> Option<TagEffect> {
        let name = tag.name().to_owned();
        let is = |lower: &str| is_name(&name, lower);
        if is("i") {
            style_add(&mut self.styles.italic);
            let angle_attribute = tag
                .attributes
                .get(1)
                .filter(|attribute| is_name(&attribute.name, "angle"));
            self.italic_angle = match angle_attribute {
                Some(_) => {
                    let angle = tag.number(1).unwrap_or(INVALID_NUMBER) as i32;
                    if !(-180..=180).contains(&angle) {
                        return None;
                    }
                    angle
                }
                None => self.face.italic_style,
            };
            self.italic_angle_stack.add(self.italic_angle);
        } else if is("mark") {
            style_add(&mut self.styles.highlight);
            let mut highlight = [255, 255, 0, 64];
            for (index, attribute) in tag.attributes.iter().enumerate() {
                if attribute.name.is_empty() {
                    break;
                }
                if is_name(&attribute.name, "mark") {
                    if attribute.value_type == ValueType::Color {
                        highlight = attribute_color(tag, 0);
                    }
                } else if attribute.name == "color" {
                    highlight = attribute_color(tag, index);
                } else if attribute.name == "padding" && padding_parameter_count(tag, index) != 4 {
                    return None;
                }
            }
            highlight[3] = highlight[3].min(self.color.alpha);
            self.highlight_stack.add(Some(highlight));
        } else if is("font-weight") {
            tag.number(0)?;
        } else if is("pos") {
            let value = tag.number(0)?;
            return Some(TagEffect::Caret(CaretCommand::MoveTo(
                self.offset(tag, value),
            )));
        } else if is("voffset") {
            let value = tag.number(0)?;
            self.baseline_offset = self.length(tag, value)?;
        } else if is("size") {
            let value = tag.number(0)?;
            self.font_size = match tag.unit {
                Unit::Pixels if matches!(tag.char_at(5), '+' | '-') => self.base_font_size + value,
                Unit::Pixels => value,
                Unit::FontUnits => self.base_font_size * value,
                Unit::Percentage => self.base_font_size * value / 100.0,
            };
            self.size_stack.add(self.font_size);
        } else if is("font") || is("material") {
            // Only the element's own font and material can be selected.
            if !matches!(tag.value(0).as_str(), "default" | "Default") {
                return None;
            }
        } else if is("space") {
            let value = tag.number(0)?;
            return Some(TagEffect::Caret(CaretCommand::Advance(
                self.length(tag, value)?,
            )));
        } else if is("alpha") {
            if tag.attributes[0].value_length != 3 {
                return None;
            }
            self.color.alpha = hex_pair(tag.char_at(7), tag.char_at(8));
        } else if is("link") {
        } else if is("align") {
            self.align = Some(match tag.value(0).as_str() {
                "left" => InlineAlign::Left,
                "right" => InlineAlign::Right,
                "center" => InlineAlign::Center,
                "justified" => InlineAlign::Justified,
                "flush" => InlineAlign::Flush,
                _ => return None,
            });
            self.align_stack.add(self.align);
        } else if is("width") {
            tag.number(0)?;
            if tag.unit == Unit::FontUnits {
                return None;
            }
        } else if is("color") {
            let length = tag.chars.len();
            if tag.char_at(6) == '#' && matches!(length, 10 | 11 | 13 | 15) {
                let rgba = hex_color(&tag.chars[7..])?;
                self.set_color(rgba);
            } else {
                let rgb = named_color(&tag.value(0))?;
                self.set_color([rgb[0], rgb[1], rgb[2], 255]);
            }
        } else if is("cspace") {
            let value = tag.number(0)?;
            self.character_spacing = self.length(tag, value)?;
        } else if is("/cspace") {
            let retracted = self.character_spacing;
            self.character_spacing = 0.0;
            if self.characters_emitted && retracted != 0.0 {
                return Some(TagEffect::Caret(CaretCommand::Advance(-retracted)));
            }
        } else if is("mspace") {
            let value = tag.number(0)?;
            self.monospace = self.length(tag, value)?;
        } else if is("indent") {
            let value = tag.number(0)?;
            self.indent = Some(self.offset(tag, value));
            self.indent_stack.add(self.indent);
        } else if is("line-indent") {
            let value = tag.number(0)?;
            self.line_indent = Some(self.offset(tag, value));
        } else if is("margin") {
            if tag.attributes[0].value_type == ValueType::Number {
                tag.number(0)?;
            } else if tag.attributes[0].value_type != ValueType::None {
                return None;
            }
        } else if is("margin-left") || is("margin-right") {
            tag.number(0)?;
        } else if is("line-height") {
            let value = tag.number(0)?;
            self.line_height = Some(match tag.unit {
                Unit::Pixels => value,
                Unit::FontUnits => value * self.font_size,
                Unit::Percentage => {
                    self.face.line_height * value / 100.0 * self.face.font_scale(self.font_size)
                }
            });
        } else if is("scale") {
            self.effect = Effect::Scale(tag.number(0)?);
        } else if is("rotate") {
            self.effect = Effect::Rotate(tag.number(0)?);
        } else {
            return None;
        }
        Some(TagEffect::Applied)
    }
}

fn padding_parameter_count(tag: &ScannedTag, index: usize) -> usize {
    let attribute = &tag.attributes[index];
    let end = (attribute.value_start + attribute.value_length).min(tag.chars.len());
    let value = &tag.chars[attribute.value_start.min(end)..end];
    if value.is_empty() {
        return 0;
    }
    value.split(|&ch| ch == ',').count()
}

/// Splits parsed runs into lines. Each line lists the runs that contribute to
/// it with the part of their text on that line; caret commands stay on the
/// line they occur in. One trailing line break does not open an empty line.
pub fn split_lines(segments: &[TextSegment]) -> Vec<Vec<LinePiece<'_>>> {
    let mut lines: Vec<Vec<LinePiece<'_>>> = vec![Vec::new()];
    for segment in segments {
        if segment.caret.is_some() {
            lines
                .last_mut()
                .expect("at least one line")
                .push(LinePiece { segment, text: "" });
            continue;
        }
        for (index, part) in segment.text.split('\n').enumerate() {
            if index > 0 {
                lines.push(Vec::new());
            }
            if !part.is_empty() {
                lines
                    .last_mut()
                    .expect("at least one line")
                    .push(LinePiece {
                        segment,
                        text: part,
                    });
            }
        }
    }
    if lines.len() > 1 && lines.last().is_some_and(Vec::is_empty) {
        lines.pop();
    }
    lines
}

/// Part of a run that falls on one line.
#[derive(Clone, Copy, Debug)]
pub struct LinePiece<'a> {
    pub segment: &'a TextSegment,
    /// Empty for a caret command.
    pub text: &'a str,
}

#[cfg(test)]
mod tests;
