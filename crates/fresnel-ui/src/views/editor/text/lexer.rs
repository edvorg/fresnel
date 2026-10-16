//! The syntax colors of YAML, line by line (research 05 section 6.9).
//!
//! The lexer reads one line with the state at its start, so a frame colors only the lines on
//! the screen, and it works on text with errors: it never refuses a line. The only state that
//! crosses a line is the body of a block scalar (`|` or `>`), whose lines are text. The parser
//! of the check, not this lexer, decides what the text means.

use std::ops::Range;

use super::buffer::Buffer;

/// The most bytes of a line that the lexer reads: the rest of a longer line, a value of
/// megabytes, takes the kind of its last token.
const READ: usize = 4096;

/// What a part of a line is, for its color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Key,
    /// A plain value.
    Text,
    /// A value in quotes.
    Quoted,
    Number,
    /// `true`, `null` and the other words of YAML.
    Word,
    Comment,
    /// A tag such as `!!binary`, an anchor or an alias.
    Tag,
    /// `:`, `-`, `|`, the brackets of a flow and the commas.
    Mark,
    /// A line of the body of a block scalar.
    Block,
    /// `---` or `...`: the start or the end of a document.
    Document,
}

/// One part of a line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Token {
    pub(crate) bytes: Range<usize>,
    pub(crate) kind: Kind,
}

/// The state at the start of a line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum State {
    #[default]
    Plain,
    /// In the body of a block scalar whose node starts at this column: each line that is
    /// empty or has more indent is text.
    Block(usize),
}

/// The state at the start of each line of a text. A scan of 8,000 lines takes less than a
/// millisecond, so the editor scans again after each change.
pub(crate) fn states(buffer: &Buffer) -> Vec<State> {
    let mut states = Vec::with_capacity(buffer.count());
    let mut state = State::Plain;
    for line in 0..buffer.count() {
        states.push(state);
        state = next(buffer.line(line), state);
    }
    states
}

/// The state after a line.
fn next(line: &str, state: State) -> State {
    if let State::Block(column) = state
        && in_block(line, column)
    {
        return state;
    }
    let mut tokens = Vec::new();
    match plain(cut(line), &mut tokens) {
        Some(column) => State::Block(column),
        None => State::Plain,
    }
}

/// The tokens of a line that starts in this state.
pub(crate) fn tokens(line: &str, state: State) -> Vec<Token> {
    let read = cut(line);
    let mut tokens = Vec::new();
    match state {
        State::Block(column) if in_block(line, column) => {
            let start = indent(line);
            if start < line.len() {
                tokens.push(Token { bytes: start..line.len(), kind: Kind::Block });
            }
            return tokens;
        }
        _ => {
            plain(read, &mut tokens);
        }
    }
    if read.len() < line.len()
        && let Some(last) = tokens.last_mut()
        && last.bytes.end == read.len()
    {
        last.bytes.end = line.len();
    }
    tokens
}

/// The part of a line that the lexer reads, cut at the start of a character.
fn cut(line: &str) -> &str {
    let mut end = line.len().min(READ);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    line.get(..end).unwrap_or_default()
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

/// True for a line of the body of a block scalar whose node starts at `column`.
fn in_block(line: &str, column: usize) -> bool {
    line.trim().is_empty() || indent(line) > column
}

/// Reads a line that is not in a block scalar. Gives the column of the node of a block scalar
/// that starts at its end.
fn plain(line: &str, tokens: &mut Vec<Token>) -> Option<usize> {
    let mut at = indent(line);
    let rest = |at: usize| line.get(at..).unwrap_or_default();
    if rest(at).starts_with('#') {
        tokens.push(Token { bytes: at..line.len(), kind: Kind::Comment });
        return None;
    }
    if at == 0 && (line.starts_with("---") || line.starts_with("...")) && ends_word(line, 3) {
        tokens.push(Token { bytes: 0..3, kind: Kind::Document });
        at = skip_spaces(line, 3);
    }
    // The node of a block scalar: the key, else the last dash of a list, else the value.
    let mut node = at;
    while rest(at).starts_with('-') && ends_word(line, at + 1) {
        tokens.push(Token { bytes: at..at + 1, kind: Kind::Mark });
        node = at;
        at = skip_spaces(line, at + 1);
    }
    if let Some(colon) = key_end(line, at) {
        tokens.push(Token { bytes: at..colon, kind: Kind::Key });
        tokens.push(Token { bytes: colon..colon + 1, kind: Kind::Mark });
        node = at;
        at = skip_spaces(line, colon + 1);
    }
    value(line, at, tokens).then_some(node)
}

/// Reads the value of a line from `at`, with a comment after it. Gives true for the start of
/// a block scalar.
fn value(line: &str, mut at: usize, tokens: &mut Vec<Token>) -> bool {
    let rest = |at: usize| line.get(at..).unwrap_or_default();
    while rest(at).starts_with(['!', '&', '*']) {
        let end = word_end(line, at);
        tokens.push(Token { bytes: at..end, kind: Kind::Tag });
        at = skip_spaces(line, end);
    }
    let text = rest(at);
    let mut block = false;
    let end = if text.starts_with(['|', '>']) {
        let end = word_end(line, at);
        tokens.push(Token { bytes: at..end, kind: Kind::Mark });
        block = true;
        end
    } else if text.starts_with(['"', '\'']) {
        let end = quoted_end(line, at);
        tokens.push(Token { bytes: at..end, kind: Kind::Quoted });
        end
    } else if text.starts_with(['[', '{']) {
        flow(line, at, tokens)
    } else {
        let end = comment_start(line, at).unwrap_or(line.len());
        let end = at + line.get(at..end).unwrap_or_default().trim_end().len();
        if end > at {
            let kind = scalar_kind(line.get(at..end).unwrap_or_default());
            tokens.push(Token { bytes: at..end, kind });
        }
        end
    };
    if let Some(comment) = comment_start(line, end) {
        tokens.push(Token { bytes: comment..line.len(), kind: Kind::Comment });
    }
    block
}

/// Reads a flow collection: its marks, and each scalar in it. Gives the end of the flow.
fn flow(line: &str, start: usize, tokens: &mut Vec<Token>) -> usize {
    let mut at = start;
    let mut depth = 0_usize;
    while let Some(character) = line.get(at..).and_then(|rest| rest.chars().next()) {
        match character {
            '[' | '{' | ']' | '}' | ',' | ':' => {
                depth = match character {
                    '[' | '{' => depth + 1,
                    ']' | '}' => depth.saturating_sub(1),
                    _ => depth,
                };
                tokens.push(Token { bytes: at..at + 1, kind: Kind::Mark });
                at += 1;
                if depth == 0 {
                    return at;
                }
            }
            '"' | '\'' => {
                let end = quoted_end(line, at);
                tokens.push(Token { bytes: at..end, kind: Kind::Quoted });
                at = end;
            }
            ' ' => at += 1,
            _ => {
                let rest = line.get(at..).unwrap_or_default();
                let length = rest.find(['[', '{', ']', '}', ',', ':']).unwrap_or(rest.len());
                let end = at + rest.get(..length).unwrap_or_default().trim_end().len();
                let kind = scalar_kind(line.get(at..end).unwrap_or_default());
                tokens.push(Token { bytes: at..end.max(at + character.len_utf8()), kind });
                at = (at + length).max(at + character.len_utf8());
            }
        }
    }
    at
}

/// The kind of a plain scalar: a word of YAML, a number, or a text.
fn scalar_kind(text: &str) -> Kind {
    const WORDS: [&str; 11] = ["true", "false", "null", "~", "yes", "no", "on", "off", "y", "n", ".inf"];
    if WORDS.iter().any(|word| text.eq_ignore_ascii_case(word)) || text.eq_ignore_ascii_case(".nan") {
        return Kind::Word;
    }
    let digits = text.strip_prefix(['-', '+']).unwrap_or(text);
    let number = digits.starts_with(|first: char| first.is_ascii_digit())
        && digits.chars().all(|character| character.is_ascii_hexdigit() || "._xoXOe+-".contains(character));
    if number { Kind::Number } else { Kind::Text }
}

/// The end of a key that starts at `at`: the place of its colon. A colon is the end of a key
/// when a space or the end of the line follows it, outside quotes and before a comment.
fn key_end(line: &str, at: usize) -> Option<usize> {
    let rest = line.get(at..)?;
    if rest.starts_with(['"', '\'']) {
        let end = quoted_end(line, at);
        return line.get(end..)?.starts_with(':').then_some(end).filter(|end| ends_word(line, end + 1));
    }
    if rest.starts_with(['[', '{', '|', '>', '!', '&', '*', '#']) {
        return None;
    }
    let end = comment_start(line, at).unwrap_or(line.len());
    let part = line.get(at..end)?;
    let colon = part.match_indices(':').map(|(index, _)| at + index).find(|colon| ends_word(line, colon + 1))?;
    Some(colon)
}

/// The end of a quoted scalar that starts at `at`, after its closing quote, or the end of the line.
fn quoted_end(line: &str, at: usize) -> usize {
    let bytes = line.as_bytes();
    let Some(&quote) = bytes.get(at) else {
        return line.len();
    };
    let mut index = at + 1;
    while let Some(&byte) = bytes.get(index) {
        if quote == b'"' && byte == b'\\' {
            index += 2;
        } else if byte == quote {
            if quote == b'\'' && bytes.get(index + 1) == Some(&b'\'') {
                index += 2;
            } else {
                return index + 1;
            }
        } else {
            index += 1;
        }
    }
    line.len()
}

/// The start of a comment at or after `at`: a `#` after a space.
fn comment_start(line: &str, at: usize) -> Option<usize> {
    let rest = line.get(at..)?;
    if rest.starts_with('#') && (at == 0 || line.as_bytes().get(at - 1) == Some(&b' ')) {
        return Some(at);
    }
    rest.find(" #").map(|index| at + index + 1)
}

/// True when the byte at `at` ends a word: a space or the end of the line.
fn ends_word(line: &str, at: usize) -> bool {
    matches!(line.as_bytes().get(at), None | Some(b' '))
}

fn word_end(line: &str, at: usize) -> usize {
    line.get(at..).and_then(|rest| rest.find(' ')).map_or(line.len(), |index| at + index)
}

fn skip_spaces(line: &str, at: usize) -> usize {
    at + line.get(at..).map_or(0, |rest| rest.len() - rest.trim_start_matches(' ').len())
}

#[cfg(test)]
mod tests;
