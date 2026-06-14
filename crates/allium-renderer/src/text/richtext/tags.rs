//! TMP 标签处理逻辑。

use super::state::RichTextParseState;
use super::types::{CaseTransform, Indent, InlineAlign, LineIndent, SizeSpec};

fn named_color(name: &str) -> Option<(u8, u8, u8)> {
    match name {
        "red" => Some((255, 0, 0)),
        "green" => Some((0, 128, 0)),
        "blue" => Some((0, 0, 255)),
        "white" => Some((255, 255, 255)),
        "black" => Some((0, 0, 0)),
        "yellow" => Some((255, 255, 0)),
        "orange" => Some((255, 128, 0)),
        "purple" => Some((160, 32, 240)),
        "grey" => Some((128, 128, 128)),
        "lightblue" => Some((173, 216, 230)),
        _ => None,
    }
}

fn parse_hex_color(hex: &str) -> Option<(u8, u8, u8)> {
    let (r, g, b, _) = parse_hex_color_alpha(hex)?;
    Some((r, g, b))
}

fn parse_hex_color_alpha(hex: &str) -> Option<(u8, u8, u8, Option<u8>)> {
    match hex.len() {
        3 => Some((
            u8::from_str_radix(&hex[0..1], 16).ok()? * 17,
            u8::from_str_radix(&hex[1..2], 16).ok()? * 17,
            u8::from_str_radix(&hex[2..3], 16).ok()? * 17,
            None,
        )),
        4 => Some((
            u8::from_str_radix(&hex[0..1], 16).ok()? * 17,
            u8::from_str_radix(&hex[1..2], 16).ok()? * 17,
            u8::from_str_radix(&hex[2..3], 16).ok()? * 17,
            Some(u8::from_str_radix(&hex[3..4], 16).ok()? * 17),
        )),
        6 => Some((
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
            None,
        )),
        8 => Some((
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
            Some(u8::from_str_radix(&hex[6..8], 16).ok()?),
        )),
        _ => None,
    }
}

fn parse_indent_value(raw: &str) -> Option<Indent> {
    let value = raw.trim_matches('#');
    if let Some(pct) = value.strip_suffix('%') {
        return parse_loose_f32(pct).map(Indent::Percent);
    }
    if let Some(em) = value.strip_suffix("em") {
        return parse_loose_f32(em).map(Indent::Em);
    }
    if let Some(px) = value.strip_suffix("px") {
        return parse_loose_f32(px).map(Indent::Pixels);
    }
    parse_loose_f32(value).map(Indent::Pixels)
}

fn parse_loose_f32(raw: &str) -> Option<f32> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = trimmed.parse::<f32>() {
        return Some(value);
    }

    let mut end = 0usize;
    for (idx, ch) in trimmed.char_indices() {
        if ch.is_ascii_digit() || matches!(ch, '+' | '-' | '.' | 'e' | 'E') {
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    if end == 0 {
        return None;
    }

    let mut candidate = trimmed[..end].trim_end_matches(['.', '+', '-', 'e', 'E']);
    while !candidate.is_empty() {
        if let Ok(value) = candidate.parse::<f32>() {
            return Some(value);
        }
        candidate = candidate.trim_end_matches(['.', '+', '-', 'e', 'E']);
        if candidate.is_empty() {
            break;
        }
        candidate = &candidate[..candidate.len().saturating_sub(1)];
    }
    None
}

impl RichTextParseState {
    pub(crate) fn handle_tag(&mut self, _tag: &str, tl: &str) -> bool {
        if let Some(v) = tl.strip_prefix("color=#") {
            if let Some((r, g, b, a)) = parse_hex_color_alpha(v) {
                self.current_color = Some((r, g, b));
                self.alpha_override = a.map(|av| av as f32 / 255.0);
                self.color_stack
                    .push((self.current_color, self.alpha_override));
            }
            return true;
        }
        if let Some(v) = tl.strip_prefix("color=") {
            if let Some(c) = named_color(v) {
                self.current_color = Some(c);
                self.alpha_override = None;
                self.color_stack
                    .push((self.current_color, self.alpha_override));
            }
            return true;
        }
        if tl == "/color" {
            if self.color_stack.len() > 1 {
                self.color_stack.pop();
            }
            let (prev_color, prev_alpha) = self.color_stack.last().copied().unwrap_or((None, None));
            self.current_color = prev_color;
            self.alpha_override = prev_alpha;
            return true;
        }
        if let Some(hex) = tl.strip_prefix('#') {
            if let Some((r, g, b, a)) = parse_hex_color_alpha(hex) {
                self.current_color = Some((r, g, b));
                self.alpha_override = a.map(|av| av as f32 / 255.0);
                self.color_stack
                    .push((self.current_color, self.alpha_override));
            }
            return true;
        }
        if let Some(v) = tl.strip_prefix("size=") {
            let v = v.trim_matches('#');
            if let Some(em) = v.strip_suffix("em") {
                if let Some(n) = parse_loose_f32(em) {
                    self.size_stack.push(SizeSpec::Em(n));
                }
            } else if let Some(pct) = v.strip_suffix('%') {
                if let Some(n) = parse_loose_f32(pct) {
                    self.size_stack.push(SizeSpec::Percent(n));
                }
            } else if let Some(rest) = v.strip_prefix('+') {
                if let Some(n) = parse_loose_f32(rest) {
                    self.size_stack.push(SizeSpec::Delta(n));
                }
            } else if let Some(rest) = v.strip_prefix('-') {
                if let Some(n) = parse_loose_f32(rest) {
                    self.size_stack.push(SizeSpec::Delta(-n));
                }
            } else if let Some(n) = parse_loose_f32(v) {
                self.size_stack.push(SizeSpec::Absolute(n));
            }
            return true;
        }
        if tl == "/size" {
            self.size_stack.pop();
            return true;
        }
        if let Some(v) = tl.strip_prefix("scale=") {
            let v = v.trim_matches('#');
            if let Some(n) = parse_loose_f32(v) {
                self.scale_stack.push(n);
            }
            return true;
        }
        if tl == "/scale" {
            self.scale_stack.pop();
            return true;
        }
        match tl {
            "noparse" => {
                self.noparse_depth += 1;
                return true;
            }
            "/noparse" => {
                self.noparse_depth = (self.noparse_depth - 1).max(0);
                return true;
            }
            "nobr" | "/nobr" => return true,
            "cr" | "br" => {
                self.push_text("\n");
                return true;
            }
            "nbsp" => {
                self.append_char('\u{00A0}');
                return true;
            }
            "zwsp" => {
                self.append_char('\u{200B}');
                return true;
            }
            "zwj" => {
                self.append_char('\u{200D}');
                return true;
            }
            "shy" => {
                self.append_char('\u{00AD}');
                return true;
            }
            _ => {}
        }
        if let Some(v) = tl.strip_prefix("space=") {
            let v = v.trim_matches('#');
            if let Ok(n) = v.parse::<f32>() {
                self.push_fixed_advance(n);
            }
            return true;
        }
        if let Some(v) = tl.strip_prefix("alpha=") {
            let v = v.trim_matches('#');
            if let Some(hex) = v.get(0..2) {
                let a = u8::from_str_radix(hex, 16).unwrap_or(255);
                self.alpha_override = Some(a as f32 / 255.0);
            }
            return true;
        }
        if let Some(v) = tl.strip_prefix("voffset=") {
            let v = v.trim_matches('#');
            if let Some(n) = parse_loose_f32(v) {
                self.voffset_stack.push(n);
            }
            return true;
        }
        if tl == "/voffset" {
            self.voffset_stack.pop();
            return true;
        }
        if let Some(v) = tl.strip_prefix("rotate=") {
            let v = v.trim_matches('#');
            if let Some(n) = parse_loose_f32(v) {
                self.rotate_stack.push(n);
            }
            return true;
        }
        if tl == "/rotate" {
            self.rotate_stack.pop();
            return true;
        }
        if let Some(v) = tl.strip_prefix("cspace=") {
            let v = v.trim_matches('#');
            if let Some(n) = parse_loose_f32(v) {
                self.cspace_override = Some(n);
            }
            return true;
        }
        if tl == "/cspace" {
            self.cspace_override = None;
            return true;
        }
        if let Some(v) = tl.strip_prefix("line-height=") {
            let v = v.trim_matches('#');
            if let Some(n) = parse_loose_f32(v) {
                self.line_height_override = Some(n);
            }
            return true;
        }
        if tl == "/line-height" {
            self.line_height_override = None;
            return true;
        }
        if let Some(v) = tl.strip_prefix("line-indent=") {
            if let Some(pct) = v.strip_suffix('%') {
                if let Some(n) = parse_loose_f32(pct) {
                    self.line_indent_override = Some(LineIndent::Percent(n));
                }
            } else if let Some(n) = parse_loose_f32(v) {
                self.line_indent_override = Some(LineIndent::Pixels(n));
            }
            return true;
        }
        if tl == "/line-indent" {
            self.line_indent_override = None;
            return true;
        }
        if let Some(v) = tl.strip_prefix("indent=") {
            if let Some(parsed) = parse_indent_value(v) {
                self.indent_stack.push(parsed);
            }
            return true;
        }
        if tl == "/indent" {
            self.indent_stack.pop();
            return true;
        }
        if let Some(v) = tl.strip_prefix("align=") {
            match v {
                "left" => self.align_stack.push(InlineAlign::Left),
                "center" => self.align_stack.push(InlineAlign::Center),
                "right" => self.align_stack.push(InlineAlign::Right),
                _ => {}
            }
            return true;
        }
        if tl == "/align" {
            self.align_stack.pop();
            return true;
        }
        if let Some(v) = tl.strip_prefix("pos=") {
            if let Some(parsed) = parse_indent_value(v) {
                self.position_stack.push(parsed);
            }
            return true;
        }
        if tl == "/pos" {
            self.position_stack.pop();
            return true;
        }
        if let Some(v) = tl.strip_prefix("mspace=") {
            let mut parts = v.split_whitespace();
            if let Some(first) = parts.next() {
                if let Some(parsed) = parse_indent_value(first) {
                    self.monospace_stack.push(parsed);
                    let mut duo = false;
                    for extra in parts {
                        let extra = extra.trim_matches('#');
                        if let Some(val) = extra.strip_prefix("duospace=") {
                            duo = val != "0";
                        }
                    }
                    self.duospace_stack.push(duo);
                }
            }
            return true;
        }
        if tl == "/mspace" {
            self.monospace_stack.pop();
            self.duospace_stack.pop();
            return true;
        }
        match tl {
            "b" => {
                self.bold_depth += 1;
                return true;
            }
            "/b" => {
                self.bold_depth = (self.bold_depth - 1).max(0);
                return true;
            }
            "i" => {
                self.italic_depth += 1;
                return true;
            }
            "/i" => {
                self.italic_depth = (self.italic_depth - 1).max(0);
                return true;
            }
            "u" => {
                self.underline_depth += 1;
                return true;
            }
            "/u" => {
                self.underline_depth = (self.underline_depth - 1).max(0);
                return true;
            }
            "s" => {
                self.strikethrough_depth += 1;
                return true;
            }
            "/s" => {
                self.strikethrough_depth = (self.strikethrough_depth - 1).max(0);
                return true;
            }
            "sub" => {
                self.subscript_depth += 1;
                return true;
            }
            "/sub" => {
                self.subscript_depth = (self.subscript_depth - 1).max(0);
                return true;
            }
            "sup" => {
                self.superscript_depth += 1;
                return true;
            }
            "/sup" => {
                self.superscript_depth = (self.superscript_depth - 1).max(0);
                return true;
            }
            "uppercase" | "allcaps" => {
                self.case_stack.push(CaseTransform::Upper);
                return true;
            }
            "/uppercase" | "/allcaps" => {
                self.case_stack.pop();
                return true;
            }
            "lowercase" => {
                self.case_stack.push(CaseTransform::Lower);
                return true;
            }
            "/lowercase" => {
                self.case_stack.pop();
                return true;
            }
            "smallcaps" => {
                self.smallcaps_depth += 1;
                return true;
            }
            "/smallcaps" => {
                self.smallcaps_depth = (self.smallcaps_depth - 1).max(0);
                return true;
            }
            "/mark" => {
                self.mark_stack.pop();
                return true;
            }
            _ => {}
        }
        if let Some(v) = tl.strip_prefix("mark=#") {
            let c = parse_hex_color(v).unwrap_or((255, 255, 0));
            let a = if v.len() >= 8 {
                u8::from_str_radix(&v[6..8], 16).unwrap_or(64)
            } else {
                64
            };
            self.mark_stack.push((c.0, c.1, c.2, a));
            return true;
        }
        if tl == "/#" {
            return false;
        }
        true
    }
}
