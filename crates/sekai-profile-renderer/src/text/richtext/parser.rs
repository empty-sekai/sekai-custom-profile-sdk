//! TMP 富文本解析主循环。

use super::state::RichTextParseState;
use super::types::TextSegment;

/// 将 TMP 富文本解析为分段列表。
pub fn parse_rich_segments(raw: &str) -> Vec<TextSegment> {
    let mut state = RichTextParseState::new();
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if state.noparse_depth() > 0 {
            if chars[i] == '<' {
                if let Some(end) = chars[i..].iter().position(|&c| c == '>') {
                    let tag: String = chars[i + 1..i + end].iter().collect();
                    let tl = tag.to_lowercase();
                    if tl == "/noparse" && state.handle_tag(&tag, &tl) {
                        i += end + 1;
                        continue;
                    }
                }
            }
            state.append_char(chars[i]);
            i += 1;
            continue;
        }

        if chars[i] == '<' {
            if let Some(end) = chars[i..].iter().position(|&c| c == '>') {
                let tag: String = chars[i + 1..i + end].iter().collect();
                let tl = tag.to_lowercase();
                if state.handle_tag(&tag, &tl) {
                    i += end + 1;
                    continue;
                }
            }
        }

        state.append_char(chars[i]);
        i += 1;
    }
    state.into_segments()
}
