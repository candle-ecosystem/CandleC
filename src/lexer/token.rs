use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Ident,
    Register,
    Function,
    SelfLit,
    Void,
    Number,
    Char,
    String,
    Bool,
    Null,
    LParen,
    RParen,
    Colon,
    Comma,
    Semicolon,
    Newline,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: Kind,
    pub value: String,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone)]
pub struct LexError {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for LexError {}
