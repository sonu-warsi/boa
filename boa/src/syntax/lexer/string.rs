use super::{Cursor, Error, Tokenizer};
use crate::syntax::ast::{Position, Span};
use crate::syntax::lexer::{Token, TokenKind};
use std::{
    char::{decode_utf16, from_u32},
    convert::TryFrom,
    io::{self, ErrorKind, Read},
    str,
};

/// String literal lexing.
///
/// Note: expects for the initializer `'` or `"` to already be consumed from the cursor.
#[derive(Debug, Clone, Copy)]
pub(super) struct StringLiteral {
    terminator: StringTerminator,
}

impl StringLiteral {
    /// Creates a new string literal lexer.
    pub(super) fn new(init: char) -> Self {
        let terminator = match init {
            '\'' => StringTerminator::SingleQuote,
            '"' => StringTerminator::DoubleQuote,
            _ => unreachable!(),
        };

        Self { terminator }
    }
}

/// Terminator for the string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StringTerminator {
    SingleQuote,
    DoubleQuote,
}

impl<R> Tokenizer<R> for StringLiteral {
    fn lex(&mut self, cursor: &mut Cursor<R>, start_pos: Position) -> Result<Token, Error>
    where
        R: Read,
    {
        let mut buf = String::new();
        loop {
            let next_chr_start = cursor.pos();
            let next_chr = cursor.next().ok_or_else(|| {
                Error::from(io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "unterminated string literal",
                ))
            })??;

            match next_chr {
                '\'' if self.terminator == StringTerminator::SingleQuote => {
                    break;
                }
                '"' if self.terminator == StringTerminator::DoubleQuote => {
                    break;
                }
                '\\' => {
                    let escape = cursor.next().ok_or_else(|| {
                        Error::from(io::Error::new(
                            ErrorKind::UnexpectedEof,
                            "unterminated escape sequence in string literal",
                        ))
                    })??;
                    if escape != '\n' {
                        let escaped_ch = match escape {
                            'n' => '\n',
                            'r' => '\r',
                            't' => '\t',
                            'b' => '\x08',
                            'f' => '\x0c',
                            '0' => '\0',
                            'x' => {
                                // TODO: optimize by getting just bytes
                                let mut nums = String::with_capacity(2);
                                for _ in 0_u8..2 {
                                    let next = cursor.next().ok_or_else(|| {
                                        Error::from(io::Error::new(
                                            ErrorKind::UnexpectedEof,
                                            "unterminated escape sequence in string literal",
                                        ))
                                    })??;
                                    nums.push(next);
                                }
                                let as_num = match u64::from_str_radix(&nums, 16) {
                                    Ok(v) => v,
                                    Err(_) => 0,
                                };
                                match from_u32(as_num as u32) {
                                    Some(v) => v,
                                    None => {
                                        return Err(Error::syntax(format!(
                                            "{}: {} is not a valid Unicode scalar value",
                                            cursor.pos(),
                                            as_num
                                        )))
                                    }
                                }
                            }
                            'u' => {
                                // There are 2 types of codepoints. Surragate codepoints and
                                // unicode codepoints. UTF-16 could be surrogate codepoints,
                                // "\uXXXX\uXXXX" which make up a single unicode codepoint. We will
                                //  need to loop to make sure we catch all UTF-16 codepoints

                                // Support \u{X..X} (Unicode Codepoint)
                                if cursor.next_is('{')? {
                                    // The biggest code point is 0x10FFFF
                                    let mut code_point = String::with_capacity(6);
                                    cursor.take_until('}', &mut code_point)?;

                                    // We know this is a single unicode codepoint, convert to u32
                                    let as_num =
                                        u32::from_str_radix(&code_point, 16).map_err(|_| {
                                            Error::syntax(
                                                "malformed Unicode character escape sequence",
                                            )
                                        })?;
                                    if as_num > 0x10_FFFF {
                                        return Err(Error::syntax("Unicode codepoint must not be greater than 0x10FFFF in escape sequence"));
                                    }
                                    char::try_from(as_num).map_err(|_| {
                                        Error::syntax("invalid Unicode escape sequence")
                                    })?
                                } else {
                                    let mut codepoints: Vec<u16> = vec![];
                                    loop {
                                        // Collect each character after \u e.g \uD83D will give "D83D"
                                        let mut code_point = [0u8; 4];
                                        cursor.fill_bytes(&mut code_point)?;

                                        // Convert to u16
                                        let as_num = match u16::from_str_radix(
                                            str::from_utf8(&code_point)
                                                .expect("the cursor returned invalid UTF-8"),
                                            16,
                                        ) {
                                            Ok(v) => v,
                                            Err(_) => 0,
                                        };

                                        codepoints.push(as_num);

                                        // Check for another UTF-16 codepoint
                                        if cursor.next_is('\\')? && cursor.next_is('u')? {
                                            continue;
                                        }
                                        break;
                                    }

                                    // codepoints length should either be 1 (unicode codepoint) or
                                    // 2 (surrogate codepoint). Rust's decode_utf16 will deal with
                                    // it regardless
                                    // TODO: do not panic with invalid code points.
                                    decode_utf16(codepoints.iter().copied())
                                        .next()
                                        .expect("Could not get next codepoint")
                                        .expect("Could not get next codepoint")
                                }
                            }
                            '\'' | '"' | '\\' => escape,
                            ch => {
                                let details = format!(
                                    "invalid escape sequence `{}` at line {}, column {}",
                                    next_chr_start.line_number(),
                                    next_chr_start.column_number(),
                                    ch
                                );
                                return Err(Error::syntax(details));
                            }
                        };
                        buf.push(escaped_ch);
                    }
                }
                next_ch => buf.push(next_ch),
            }
        }

        Ok(Token::new(
            TokenKind::string_literal(buf),
            Span::new(start_pos, cursor.pos()),
        ))
    }
}