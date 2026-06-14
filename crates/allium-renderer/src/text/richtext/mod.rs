//! TMP 富文本解析模块。

mod parser;
mod state;
mod tags;
#[cfg(test)]
mod tests;
mod types;

pub use parser::parse_rich_segments;
pub use types::{CaseTransform, Indent, InlineAlign, LineIndent, SizeSpec, TextSegment};
