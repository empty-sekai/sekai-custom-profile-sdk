use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NumericTextRun {
    pub text: String,
    pub plain_start: u32,
    pub plain_end: u32,
}

pub fn strip_tmp_tags(source: &str) -> String {
    let mut plain = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '<' {
            plain.push(ch);
            continue;
        }
        let mut candidate = String::from('<');
        let mut closed = false;
        for next in chars.by_ref() {
            candidate.push(next);
            if next == '>' {
                closed = true;
                break;
            }
        }
        if !closed {
            plain.push_str(&candidate);
        }
    }
    plain
}

pub fn numeric_text_runs(source: &str) -> Vec<NumericTextRun> {
    let plain = strip_tmp_tags(source);
    let chars = plain.chars().collect::<Vec<_>>();
    let mut runs = Vec::new();
    let mut cursor = 0;
    while cursor < chars.len() {
        if !chars[cursor].is_ascii_digit() {
            cursor += 1;
            continue;
        }
        let start = cursor;
        while cursor < chars.len() && chars[cursor].is_ascii_digit() {
            cursor += 1;
        }
        runs.push(NumericTextRun {
            text: chars[start..cursor].iter().collect(),
            plain_start: start as u32,
            plain_end: cursor as u32,
        });
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tmp_tags_do_not_split_contiguous_ascii_digits() {
        assert_eq!(
            numeric_text_runs("<color=#fff>12</color><b>34</b>"),
            vec![NumericTextRun {
                text: "1234".into(),
                plain_start: 0,
                plain_end: 4,
            }]
        );
    }

    #[test]
    fn visible_symbols_and_newlines_split_runs_and_leading_zeroes_survive() {
        assert_eq!(
            numeric_text_runs("0012-34\n56"),
            vec![
                NumericTextRun {
                    text: "0012".into(),
                    plain_start: 0,
                    plain_end: 4
                },
                NumericTextRun {
                    text: "34".into(),
                    plain_start: 5,
                    plain_end: 7
                },
                NumericTextRun {
                    text: "56".into(),
                    plain_start: 8,
                    plain_end: 10
                },
            ]
        );
    }
}
