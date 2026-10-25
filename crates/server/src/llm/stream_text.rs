//! Streaming extraction of one JSON string field.
//!
//! Structured output is still streamed to the UI. The raw JSON is buffered for
//! the final `serde` validation; this helper only mirrors the value of the
//! `message` (or `explanation`) field while it arrives. It is explicitly not
//! used to parse the final answer.

#[derive(Debug, Clone, Copy)]
enum State {
    Search,
    AfterKey,
    BeforeValue,
    InString,
    Escape,
    Unicode { value: u32, digits: u8 },
    AfterHigh { high: u32 },
    ExpectUnicodeStart { high: u32 },
    UnicodeLow { high: u32, value: u32, digits: u8 },
    Done,
}

#[derive(Debug)]
pub struct JsonStringFieldStreamer {
    key: Vec<char>,
    matched: usize,
    state: State,
}

impl JsonStringFieldStreamer {
    pub fn new(field: &str) -> Self {
        let mut key = String::new();
        key.push('"');
        key.push_str(field);
        key.push('"');
        Self {
            key: key.chars().collect(),
            matched: 0,
            state: State::Search,
        }
    }

    /// Feed a raw provider chunk and return the newly decoded field text.
    pub fn feed(&mut self, chunk: &str) -> String {
        let mut output = String::new();
        for ch in chunk.chars() {
            match self.state {
                State::Search => self.feed_search(ch),
                State::AfterKey => {
                    if ch.is_whitespace() {
                        continue;
                    }
                    if ch == ':' {
                        self.state = State::BeforeValue;
                    } else {
                        self.state = State::Search;
                        self.matched = 0;
                    }
                }
                State::BeforeValue => {
                    if ch.is_whitespace() {
                        continue;
                    }
                    if ch == '"' {
                        self.state = State::InString;
                    } else {
                        self.state = State::Done;
                    }
                }
                State::InString => match ch {
                    '\\' => self.state = State::Escape,
                    '"' => self.state = State::Done,
                    other => output.push(other),
                },
                State::Escape => match ch {
                    '"' => {
                        output.push('"');
                        self.state = State::InString;
                    }
                    '\\' => {
                        output.push('\\');
                        self.state = State::InString;
                    }
                    '/' => {
                        output.push('/');
                        self.state = State::InString;
                    }
                    'b' => {
                        output.push('\u{0008}');
                        self.state = State::InString;
                    }
                    'f' => {
                        output.push('\u{000c}');
                        self.state = State::InString;
                    }
                    'n' => {
                        output.push('\n');
                        self.state = State::InString;
                    }
                    'r' => {
                        output.push('\r');
                        self.state = State::InString;
                    }
                    't' => {
                        output.push('\t');
                        self.state = State::InString;
                    }
                    'u' => {
                        self.state = State::Unicode {
                            value: 0,
                            digits: 0,
                        };
                    }
                    _ => self.state = State::Done,
                },
                State::Unicode { value, digits } => {
                    if let Some(hex) = ch.to_digit(16) {
                        let value = (value << 4) | hex;
                        let digits = digits + 1;
                        if digits == 4 {
                            self.complete_unicode(value, &mut output);
                        } else {
                            self.state = State::Unicode { value, digits };
                        }
                    } else {
                        self.state = State::Done;
                    }
                }
                State::AfterHigh { high } => {
                    if ch == '\\' {
                        self.state = State::ExpectUnicodeStart { high };
                    } else {
                        output.push('\u{FFFD}');
                        self.state = State::Done;
                    }
                }
                State::ExpectUnicodeStart { high } => {
                    if ch == 'u' {
                        self.state = State::UnicodeLow {
                            high,
                            value: 0,
                            digits: 0,
                        };
                    } else {
                        output.push('\u{FFFD}');
                        self.state = State::Done;
                    }
                }
                State::UnicodeLow {
                    high,
                    value,
                    digits,
                } => {
                    if let Some(hex) = ch.to_digit(16) {
                        let value = (value << 4) | hex;
                        let digits = digits + 1;
                        if digits == 4 {
                            if !(0xDC00..=0xDFFF).contains(&value) {
                                output.push('\u{FFFD}');
                                self.state = State::Done;
                                continue;
                            }
                            let codepoint = 0x10000 + ((high - 0xD800) << 10) + (value - 0xDC00);
                            if let Some(decoded) = char::from_u32(codepoint) {
                                output.push(decoded);
                                self.state = State::InString;
                            } else {
                                self.state = State::Done;
                            }
                        } else {
                            self.state = State::UnicodeLow {
                                high,
                                value,
                                digits,
                            };
                        }
                    } else {
                        self.state = State::Done;
                    }
                }
                State::Done => break,
            }
        }
        output
    }

    fn feed_search(&mut self, ch: char) {
        if ch == self.key[self.matched] {
            self.matched += 1;
            if self.matched == self.key.len() {
                self.matched = 0;
                self.state = State::AfterKey;
            }
        } else {
            self.matched = if ch == self.key[0] { 1 } else { 0 };
        }
    }

    fn complete_unicode(&mut self, value: u32, output: &mut String) {
        match value {
            0xD800..=0xDBFF => self.state = State::AfterHigh { high: value },
            0xDC00..=0xDFFF => {
                output.push('\u{FFFD}');
                self.state = State::InString;
            }
            _ => {
                if let Some(decoded) = char::from_u32(value) {
                    output.push(decoded);
                } else {
                    output.push('\u{FFFD}');
                }
                self.state = State::InString;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_value_across_arbitrary_chunks() {
        let raw = r#"{"agent":"proponent","message":"Привет, мир!","x":1}"#;
        let mut streamer = JsonStringFieldStreamer::new("message");
        let mut text = String::new();
        let chars: Vec<char> = raw.chars().collect();
        for chunk in chars.chunks(3) {
            let chunk: String = chunk.iter().collect();
            text.push_str(&streamer.feed(&chunk));
        }
        assert_eq!(text, "Привет, мир!");
    }

    #[test]
    fn decodes_escapes_and_unicode() {
        let raw = r#"{"message":"a\nb\"c\u043f\u0440\u0438\u0432\u0435\u0442"}"#;
        let mut streamer = JsonStringFieldStreamer::new("message");
        assert_eq!(streamer.feed(raw), "a\nb\"cпривет");
    }

    #[test]
    fn decodes_surrogate_pairs() {
        let raw = r#"{"message":"\ud83d\ude00 ok"}"#;
        let mut streamer = JsonStringFieldStreamer::new("message");
        assert_eq!(streamer.feed(raw), "😀 ok");
    }

    #[test]
    fn stops_at_closing_quote() {
        let raw = r#"{"message":"one","other":"two"}"#;
        let mut streamer = JsonStringFieldStreamer::new("message");
        assert_eq!(streamer.feed(raw), "one");
    }

    #[test]
    fn does_nothing_when_field_is_absent() {
        let raw = r#"{"other":"value"}"#;
        let mut streamer = JsonStringFieldStreamer::new("message");
        assert_eq!(streamer.feed(raw), "");
    }
}
