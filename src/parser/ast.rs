#[derive(Debug, Clone)]
pub enum Operand {
    Register(u8),
    Ident(String),
    Number(String),
    Char(char),
    String(String),
    Bool(bool),
    Null,
    SelfLit,
    Void,
}

#[derive(Debug, Clone)]
pub struct Instruction {
    pub name: String,
    pub operands: Vec<Operand>,
    pub line: usize,
    pub column: usize,
    pub operand_columns: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub params: u8,
    pub captures: u8,
    pub instructions: Vec<Instruction>,
    pub line: usize,
    pub column: usize,
    pub params_column: usize,
    pub captures_column: usize,
}

#[derive(Debug, Clone)]
pub struct Program {
    pub functions: Vec<Function>,
}
