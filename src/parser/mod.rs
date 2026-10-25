mod ast;

mod error;

mod parser;

pub use ast::{Function, Instruction, Operand, Program};

pub use error::ParseError;

pub use parser::parse;
