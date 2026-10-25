use super::{
    ast::{Function, Instruction, Operand, Program},
    error::ParseError,
};
use crate::lexer::{Kind, Token};

pub fn parse(tokens: &[Token]) -> Result<Program, ParseError> {
    Parser { t: tokens, p: 0 }.program()
}

struct Parser<'a> {
    t: &'a [Token],
    p: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> &Token {
        &self.t[self.p]
    }

    fn previous(&self) -> &Token {
        &self.t[self.p.saturating_sub(1)]
    }

    fn adv(&mut self) -> Token {
        let t = self.peek().clone();
        if t.kind != Kind::Eof {
            self.p += 1;
        }
        t
    }

    fn check(&self, k: Kind) -> bool {
        self.peek().kind == k
    }

    fn eat(&mut self, k: Kind) -> bool {
        if self.check(k) {
            self.adv();
            true
        } else {
            false
        }
    }

    fn need(&mut self, k: Kind, msg: &str) -> Result<Token, ParseError> {
        if self.check(k) {
            Ok(self.adv())
        } else {
            Err(self.error(self.peek(), msg))
        }
    }

    fn error(&self, t: &Token, msg: &str) -> ParseError {
        ParseError {
            message: msg.into(),
            line: t.line,
            column: t.column,
        }
    }

    fn skip(&mut self) {
        while self.eat(Kind::Newline) {}
    }

    fn program(mut self) -> Result<Program, ParseError> {
        let mut fs = Vec::new();
        self.skip();
        while !self.check(Kind::Eof) {
            if fs.len() >= 65535 {
                return Err(self.error(
                    self.peek(),
                    "A module cannot contain more than 65535 functions",
                ));
            }
            fs.push(self.function()?);
            self.skip();
        }
        Ok(Program { functions: fs })
    }

    fn function(&mut self) -> Result<Function, ParseError> {
        self.need(Kind::Function, "Expected 'function'")?;
        let n = self.need(Kind::Ident, "Expected function name")?;
        self.need(Kind::LParen, "Expected '(' after function name")?;
        let ct = self.need(Kind::Number, "Expected parameter count")?;
        let params = ct
            .value
            .parse::<i32>()
            .map_err(|_| self.error(&ct, "Parameter count must be an integer"))?;
        if params < 0 {
            return Err(self.error(&ct, "Parameter count cannot be negative"));
        }
        if params > candlevm::constants::MAX_FUNCTION_PARAMETERS as i32 {
            return Err(self.error(
                &ct,
                &format!(
                    "A function cannot have more than {} parameters",
                    candlevm::constants::MAX_FUNCTION_PARAMETERS
                ),
            ));
        }
        self.need(
            Kind::Semicolon,
            "Expected ';' between parameter and capture counts",
        )?;
        let captures_token = self.need(Kind::Number, "Expected capture count")?;
        let captures = captures_token
            .value
            .parse::<i32>()
            .map_err(|_| self.error(&captures_token, "Capture count must be an integer"))?;
        if captures < 0 {
            return Err(self.error(&captures_token, "Capture count cannot be negative"));
        }
        if captures > candlevm::constants::MAX_CAPTURE_COUNT as i32 {
            return Err(self.error(
                &captures_token,
                &format!(
                    "A function cannot have more than {} captures",
                    candlevm::constants::MAX_CAPTURE_COUNT
                ),
            ));
        }
        self.need(Kind::RParen, "Expected ')' after capture count")?;
        self.need(Kind::Colon, "Expected ':' after function declaration")?;
        self.need(Kind::Newline, "Expected newline after function declaration")?;
        let mut ins = Vec::new();
        self.skip();
        while !self.check(Kind::Function) && !self.check(Kind::Eof) {
            ins.push(self.instruction()?);
            self.skip();
        }
        if ins.is_empty() {
            return Err(self.error(&n, "Function body cannot be empty"));
        }
        Ok(Function {
            name: n.value,
            params: params as u8,
            captures: captures as u8,
            instructions: ins,
            line: n.line,
            column: n.column,
            params_column: ct.column,
            captures_column: captures_token.column,
        })
    }

    fn instruction(&mut self) -> Result<Instruction, ParseError> {
        let n = self.need(Kind::Ident, "Expected instruction name")?;
        let mut ops = Vec::new();
        let mut operand_columns = Vec::new();
        let mut expect = true;
        while !matches!(self.peek().kind, Kind::Newline | Kind::Eof | Kind::Function) {
            if self.eat(Kind::Comma) {
                if expect || ops.is_empty() {
                    return Err(self.error(self.previous(), "Unexpected comma"));
                }
                expect = true;
                continue;
            }
            if !expect && self.previous().kind == Kind::Comma {
                return Err(self.error(self.peek(), "Expected operand after comma"));
            }
            operand_columns.push(self.peek().column);
            ops.push(self.operand()?);
            expect = false;
        }
        if expect && !ops.is_empty() {
            return Err(self.error(self.previous(), "Expected operand after comma"));
        }
        if self.check(Kind::Function) {
            return Err(self.error(self.peek(), "Expected newline before next function"));
        }
        self.eat(Kind::Newline);
        Ok(Instruction {
            name: n.value,
            operands: ops,
            line: n.line,
            column: n.column,
            operand_columns,
        })
    }

    fn operand(&mut self) -> Result<Operand, ParseError> {
        let t = self.adv();
        Ok(match t.kind {
            Kind::Register => Operand::Register(t.value[1..].parse().unwrap_or(255)),
            Kind::Ident => Operand::Ident(t.value),
            Kind::Number => Operand::Number(t.value),
            Kind::Char => Operand::Char(
                t.value
                    .chars()
                    .next()
                    .ok_or_else(|| self.error(&t, "Invalid character literal"))?,
            ),
            Kind::String => Operand::String(t.value),
            Kind::Bool => Operand::Bool(t.value == "true"),
            Kind::Null => Operand::Null,
            Kind::SelfLit => Operand::SelfLit,
            Kind::Void => Operand::Void,
            _ => return Err(self.error(&t, "Expected operand")),
        })
    }
}
