use crate::parser::{Instruction, Operand};
use std::fmt;

#[derive(Debug, Clone)]
pub struct CompileError {
    pub message: String,
    pub line: i32,
    pub operand: i32,
}

impl CompileError {
    pub(super) fn new(s: impl Into<String>, operand: i32) -> Self {
        Self {
            message: s.into(),
            line: 0,
            operand,
        }
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CompileError {}

#[derive(Default)]
pub struct Writer {
    pub data: Vec<u8>,
}

impl Writer {
    pub(super) fn byte(&mut self, v: usize) {
        self.data.push(v as u8)
    }

    pub(super) fn bytes(&mut self, b: &[u8]) {
        self.data.extend_from_slice(b)
    }

    pub(super) fn i32(&mut self, v: i32) {
        self.bytes(&v.to_le_bytes())
    }

    pub(super) fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes())
    }

    pub(super) fn string(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.bytes(s.as_bytes())
    }
}

pub(super) fn reg(o: &[Operand], i: usize) -> Result<u8, CompileError> {
    match o.get(i) {
        Some(Operand::Register(n)) => Ok(*n),
        Some(Operand::SelfLit) => Ok(0),
        Some(Operand::Void) => Ok(255),
        _ => Err(CompileError::new(
            "Expected a register, 'self' or 'void'",
            i as i32,
        )),
    }
}

pub(super) fn string(o: &[Operand], i: usize) -> Result<&str, CompileError> {
    if let Some(Operand::String(s)) = o.get(i) {
        Ok(s)
    } else {
        Err(CompileError::new("Expected a string literal", i as i32))
    }
}

pub(super) fn integer(o: &[Operand], i: usize, bits: u32) -> Result<i64, CompileError> {
    if let Some(Operand::Number(s)) = o.get(i) {
        let (negative, unsigned) = s
            .strip_prefix('-')
            .map_or((false, s.as_str()), |v| (true, v));
        let parsed = if let Some(h) = unsigned
            .strip_prefix("0x")
            .or_else(|| unsigned.strip_prefix("0X"))
        {
            u128::from_str_radix(h, 16).ok().and_then(|v| {
                if negative {
                    i128::try_from(v).ok().map(|x| -x)
                } else {
                    i128::try_from(v).ok()
                }
            })
        } else {
            s.parse::<i128>().ok()
        };
        if let Some(v) = parsed {
            let (lo, hi) = if bits == 32 {
                (i32::MIN as i128, i32::MAX as i128)
            } else {
                (i64::MIN as i128, i64::MAX as i128)
            };
            if (lo..=hi).contains(&v) {
                return Ok(v as i64);
            }
        }
        return Err(CompileError::new(
            format!("Invalid {}-bit integer literal: {s}", bits),
            i as i32,
        ));
    }
    Err(CompileError::new("Expected a number literal", i as i32))
}

pub(super) fn numeric_value(s: &str) -> Result<f64, ()> {
    let (negative, unsigned) = s.strip_prefix('-').map_or((false, s), |v| (true, v));
    if let Some(h) = unsigned
        .strip_prefix("0x")
        .or_else(|| unsigned.strip_prefix("0X"))
    {
        let mut value = 0.0f64;
        for c in h.chars() {
            value = value * 16.0 + c.to_digit(16).ok_or(())? as f64;
        }
        if negative {
            value = -value;
        }
        if value.is_finite() {
            Ok(value)
        } else {
            Err(())
        }
    } else {
        let value = s.parse::<f64>().map_err(|_| ())?;
        if value.is_finite() {
            Ok(value)
        } else {
            Err(())
        }
    }
}

pub(super) fn exact(i: &Instruction, n: usize) -> Result<(), CompileError> {
    if i.operands.len() == n {
        Ok(())
    } else {
        Err(CompileError::new(
            format!("'{}' expects exactly {n} operands", i.name),
            if i.operands.len() > n { n as i32 } else { -1 },
        ))
    }
}

pub(super) fn regs(
    i: &Instruction,
    w: &mut Writer,
    op: u8,
    ids: &[usize],
) -> Result<(), CompileError> {
    w.byte(op as usize);
    for &idx in ids {
        w.byte(reg(&i.operands, idx)? as usize);
    }
    Ok(())
}
