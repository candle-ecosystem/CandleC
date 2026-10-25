use super::{Kind, LexError, Token};

pub fn tokenize(source: &str) -> Result<Vec<Token>, LexError> {
    let chars: Vec<char> = source.chars().collect();
    let mut i = 0usize;
    let (mut line, mut col) = (1usize, 1usize);
    let mut out = Vec::new();
    let err = |message: String, line, column| LexError {
        message,
        line,
        column,
    };
    while i < chars.len() {
        let c = chars[i];
        if matches!(c, ' ' | '\t' | '\r') {
            i += 1;
            col += 1;
            continue;
        }
        if c == '\n' {
            out.push(Token {
                kind: Kind::Newline,
                value: "\n".into(),
                line,
                column: col,
            });
            i += 1;
            line += 1;
            col = 1;
            continue;
        }
        if c == '#' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
                col += 1;
            }
            continue;
        }
        let start_col = col;
        let single = match c {
            '(' => Some(Kind::LParen),
            ')' => Some(Kind::RParen),
            ':' => Some(Kind::Colon),
            ',' => Some(Kind::Comma),
            ';' => Some(Kind::Semicolon),
            _ => None,
        };
        if let Some(kind) = single {
            out.push(Token {
                kind,
                value: c.to_string(),
                line,
                column: col,
            });
            i += 1;
            col += 1;
            continue;
        }
        if c == '"' || c == '\'' {
            let quote = c;
            i += 1;
            col += 1;
            let mut value = String::new();
            loop {
                if i >= chars.len() || chars[i] == '\n' {
                    return Err(err(
                        if quote == '"' {
                            "Unterminated string literal"
                        } else {
                            "Character literal must contain one character"
                        }
                        .into(),
                        line,
                        start_col,
                    ));
                }
                if chars[i] == quote {
                    i += 1;
                    col += 1;
                    break;
                }
                if chars[i] == '\\' {
                    i += 1;
                    col += 1;
                    if i >= chars.len() {
                        return Err(err("Incomplete escape sequence".into(), line, col));
                    }
                    let e = chars[i];
                    value.push(match e {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        '0' => '\0',
                        '\\' => '\\',
                        '\'' => '\'',
                        '"' => '"',
                        _ => {
                            return Err(err(
                                format!("Unknown escape sequence '\\\\{e}'"),
                                line,
                                col,
                            ));
                        }
                    });
                    i += 1;
                    col += 1;
                } else {
                    value.push(chars[i]);
                    i += 1;
                    col += 1;
                }
            }
            if quote == '\'' && value.chars().count() != 1 {
                return Err(err(
                    "Character literal must contain one character".into(),
                    line,
                    start_col,
                ));
            }
            out.push(Token {
                kind: if quote == '"' {
                    Kind::String
                } else {
                    Kind::Char
                },
                value,
                line,
                column: start_col,
            });
            continue;
        }
        if c.is_ascii_digit() || (c == '-' && chars.get(i + 1).is_some_and(|x| x.is_ascii_digit()))
        {
            let start = i;
            if c == '-' {
                i += 1;
                col += 1;
            }
            if chars.get(i) == Some(&'0') && matches!(chars.get(i + 1), Some('x' | 'X')) {
                i += 2;
                col += 2;
                let d = i;
                while i < chars.len() && chars[i].is_ascii_hexdigit() {
                    i += 1;
                    col += 1;
                }
                if d == i {
                    return Err(err(
                        "Hexadecimal number must contain at least one digit".into(),
                        line,
                        start_col,
                    ));
                }
                if i < chars.len() && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '.'))
                {
                    return Err(err(
                        "Invalid hexadecimal number literal".into(),
                        line,
                        start_col,
                    ));
                }
            } else {
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                    col += 1;
                }
                if chars.get(i) == Some(&'.')
                    && chars.get(i + 1).is_some_and(|x| x.is_ascii_digit())
                {
                    i += 1;
                    col += 1;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                        col += 1;
                    }
                }
            }
            out.push(Token {
                kind: Kind::Number,
                value: chars[start..i].iter().collect(),
                line,
                column: start_col,
            });
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len()
                && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '.' | '-'))
            {
                i += 1;
                col += 1;
            }
            let value: String = chars[start..i].iter().collect();
            let kind = match value.as_str() {
                "function" => Kind::Function,
                "self" => Kind::SelfLit,
                "void" => Kind::Void,
                "null" => Kind::Null,
                "true" | "false" => Kind::Bool,
                _ if value.len() > 1
                    && value.starts_with('r')
                    && value[1..].chars().all(|x| x.is_ascii_digit()) =>
                {
                    let n = value[1..]
                        .parse::<u16>()
                        .map_err(|_| err(format!("Invalid register '{value}'"), line, start_col))?;
                    if n > 255 {
                        return Err(err(
                            "Register index must be between 0 and 255".into(),
                            line,
                            start_col,
                        ));
                    }
                    Kind::Register
                }
                _ => Kind::Ident,
            };
            out.push(Token {
                kind,
                value,
                line,
                column: start_col,
            });
            continue;
        }
        return Err(err(format!("Unexpected character '{c}'"), line, col));
    }
    out.push(Token {
        kind: Kind::Eof,
        value: String::new(),
        line,
        column: col,
    });
    Ok(out)
}
