//! The query text and the positions the tokenizer and parser report into it.

use sqlparser::{
    keywords::Keyword,
    tokenizer::{Location, Span, Token, TokenWithSpan, Tokenizer, Whitespace},
};

use super::Dialect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Range {
    pub(super) start: usize,
    pub(super) end: usize,
}

impl Range {
    pub(super) fn contains(self, other: Self) -> bool {
        self.start <= other.start && other.end <= self.end
    }

    pub(super) fn contains_position(self, position: usize) -> bool {
        self.start <= position && position < self.end
    }

    pub(super) fn len(self) -> usize {
        self.end - self.start
    }
}

pub(super) struct TokenAt {
    pub(super) token: Token,
    pub(super) range: Range,
    pub(super) depth: usize,
}

pub(super) struct MultilineTokens {
    /// Literals and quoted identifiers spanning several lines: a condition marker cannot be
    /// appended to a line that ends inside one, so gated structures may not contain them.
    pub(super) literals: Vec<Range>,
    /// Block comments spanning several lines: inert SQL, blanked out inside gated structures.
    pub(super) comments: Vec<Range>,
}

/// The query text with the byte offset of every line start, the tokenizer's tokens, and the
/// significant tokens with their parenthesis depth. Converts the line/column positions the
/// tokenizer and parser report into byte ranges.
pub(super) struct Source<'s> {
    pub(super) sql: &'s str,
    pub(super) locations: Vec<usize>,
    pub(super) raw: Vec<TokenWithSpan>,
    pub(super) tokens: Vec<TokenAt>,
}

impl<'s> Source<'s> {
    pub(super) fn tokenize(sql: &'s str, dialect: Dialect) -> Result<Self, String> {
        let raw = Tokenizer::new(dialect.sqlparser(), sql)
            .tokenize_with_location()
            .map_err(|error| format!("dynamic filters could not tokenize query text: {error}"))?;
        let locations = std::iter::once(0)
            .chain(
                sql.bytes()
                    .enumerate()
                    .filter_map(|(index, byte)| (byte == b'\n').then_some(index + 1)),
            )
            .collect();
        let mut source = Self {
            sql,
            locations,
            raw,
            tokens: Vec::new(),
        };
        source.tokens = source.significant_tokens();
        Ok(source)
    }

    fn significant_tokens(&self) -> Vec<TokenAt> {
        let mut depth = 0_usize;
        self.raw
            .iter()
            .filter_map(|token| {
                let range = self.token_range(token);
                match &token.token {
                    Token::Whitespace(_) => None,
                    Token::RParen => {
                        depth = depth.saturating_sub(1);
                        Some(TokenAt {
                            token: Token::RParen,
                            range,
                            depth,
                        })
                    }
                    Token::LParen => {
                        let result = TokenAt {
                            token: Token::LParen,
                            range,
                            depth,
                        };
                        depth += 1;
                        Some(result)
                    }
                    token => Some(TokenAt {
                        token: token.clone(),
                        range,
                        depth,
                    }),
                }
            })
            .collect()
    }

    pub(super) fn token_range(&self, token: &TokenWithSpan) -> Range {
        Range {
            start: self.offset(token.span.start),
            end: self.offset(token.span.end),
        }
    }

    pub(super) fn multiline_tokens(&self) -> MultilineTokens {
        let mut tokens = MultilineTokens {
            literals: Vec::new(),
            comments: Vec::new(),
        };
        for token in &self.raw {
            let range = self.token_range(token);
            if !self.sql[range.start..range.end].contains('\n') {
                continue;
            }
            match &token.token {
                Token::Whitespace(Whitespace::MultiLineComment(_)) => tokens.comments.push(range),
                Token::Whitespace(_) => {}
                _ => tokens.literals.push(range),
            }
        }
        tokens
    }

    pub(super) fn offset(&self, location: Location) -> usize {
        let Some(start) = location
            .line
            .checked_sub(1)
            .and_then(|line| self.locations.get(line as usize))
            .copied()
        else {
            return self.sql.len();
        };
        let end = self.line_end(start);
        self.sql[start..end]
            .char_indices()
            .nth(location.column.saturating_sub(1) as usize)
            .map_or(end, |(offset, _)| start + offset)
    }

    pub(super) fn range(&self, span: Span) -> Option<Range> {
        (span.start.line != 0 && span.end.line != 0).then(|| Range {
            start: self.offset(span.start),
            end: self.offset(span.end),
        })
    }

    pub(super) fn line_of(&self, offset: usize) -> usize {
        self.locations.partition_point(|start| *start <= offset) - 1
    }

    /// Byte offset of the newline ending the line containing `offset`, or the end of the text.
    pub(super) fn line_end(&self, offset: usize) -> usize {
        self.sql[offset..]
            .find('\n')
            .map_or(self.sql.len(), |end| offset + end)
    }

    /// The range from the first to the last significant token inside `range`.
    pub(super) fn surface(&self, range: Range) -> Range {
        let mut tokens = self
            .tokens
            .iter()
            .filter(|token| range.contains(token.range));
        let first = tokens.next();
        let last = tokens.next_back().or(first);
        Range {
            start: first.map_or(range.start, |token| token.range.start),
            end: last.map_or(range.end, |token| token.range.end),
        }
    }

    pub(super) fn trim(&self, mut start: usize, mut end: usize) -> Range {
        let bytes = self.sql.as_bytes();
        while start < end && bytes[start].is_ascii_whitespace() {
            start += 1;
        }
        while start < end && bytes[end - 1].is_ascii_whitespace() {
            end -= 1;
        }
        Range { start, end }
    }
}

pub(super) fn keyword(token: &Token) -> Keyword {
    match token {
        Token::Word(word) => word.keyword,
        _ => Keyword::NoKeyword,
    }
}

pub(super) fn first_non_whitespace(text: &str, start: usize, end: usize) -> Option<usize> {
    text[start..end]
        .char_indices()
        .find(|(_, character)| !character.is_whitespace())
        .map(|(offset, _)| start + offset)
}

pub(super) fn previous_non_whitespace(text: &str, start: usize) -> Option<usize> {
    text[..start]
        .char_indices()
        .rev()
        .find(|(_, character)| !character.is_whitespace())
        .map(|(offset, _)| offset)
}
