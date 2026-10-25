use super::{
    CompileError, Instruction, Operand, Writer,
    instruction::{exact, integer, numeric_value, reg, regs, string},
    simple::op_info,
};
use candlevm::opcode::Opcode;
use candlevm::value::Value;

pub(super) fn encode(
    instruction: &Instruction,
    writer: &mut Writer,
    current: usize,
    offsets: &[usize],
    measure: bool,
) -> Result<(), CompileError> {
    match instruction.name.as_str() {
        "load_const" => {
            exact(instruction, 2)?;
            writer.byte(Opcode::LoadConst as usize);
            writer.byte(reg(&instruction.operands, 0)? as usize);
            match &instruction.operands[1] {
                Operand::Null => Value::Null.serialize(&mut writer.data),
                Operand::Bool(v) => Value::Bool(*v).serialize(&mut writer.data),
                Operand::Char(c) => Value::Char(*c).serialize(&mut writer.data),
                Operand::Number(s) => {
                    let x = numeric_value(s).map_err(|_| {
                        CompileError::new(format!("Invalid number literal: {s}"), 1)
                    })?;
                    if !s.contains('.') && !s.starts_with('-') && x <= 255.0 {
                        Value::Byte(x as u8).serialize(&mut writer.data)
                    } else {
                        Value::Number(x).serialize(&mut writer.data)
                    }
                }
                _ => {
                    return Err(CompileError::new(
                        "Expected a literal value (null, boolean, char or number)",
                        1,
                    ));
                }
            }
            Ok(())
        }
        "load_func" => {
            exact(instruction, 3)?;
            writer.byte(Opcode::LoadFunction as usize);
            writer.byte(reg(&instruction.operands, 0)? as usize);
            writer.byte(reg(&instruction.operands, 1)? as usize);
            writer.string(string(&instruction.operands, 2)?);
            Ok(())
        }
        "str_new" => {
            exact(instruction, 2)?;
            writer.byte(Opcode::NewString as usize);
            writer.byte(reg(&instruction.operands, 0)? as usize);
            writer.string(string(&instruction.operands, 1)?);
            Ok(())
        }
        "load_mod" | "load_global" | "load_res" => {
            exact(instruction, 2)?;
            let ix = if instruction.name == "load_mod" {
                Opcode::LoadModule as usize
            } else if instruction.name == "load_global" {
                Opcode::LoadGlobal as usize
            } else {
                Opcode::LoadResource as usize
            };
            writer.byte(ix);
            writer.byte(reg(&instruction.operands, 0)? as usize);
            writer.string(string(&instruction.operands, 1)?);
            Ok(())
        }
        "store_global" => {
            exact(instruction, 2)?;
            writer.byte(Opcode::StoreGlobal as usize);
            writer.string(string(&instruction.operands, 1)?);
            writer.byte(reg(&instruction.operands, 0)? as usize);
            Ok(())
        }
        "call" | "pcall" => {
            let min = if instruction.name == "call" { 2 } else { 3 };
            if instruction.operands.len() < min {
                return Err(CompileError::new(
                    format!("'{}' expects at least {min} operands", instruction.name),
                    -1,
                ));
            }
            let fixed_operand_count = if instruction.name == "call" { 2 } else { 3 };
            let parameter_count = instruction.operands.len() - fixed_operand_count;
            if parameter_count > candlevm::constants::MAX_FUNCTION_PARAMETERS as usize {
                return Err(CompileError::new(
                    format!(
                        "'{}' accepts at most {} parameters",
                        instruction.name,
                        candlevm::constants::MAX_FUNCTION_PARAMETERS
                    ),
                    instruction.line as i32,
                ));
            }
            writer.byte(if instruction.name == "call" {
                Opcode::Call as usize
            } else {
                Opcode::Pcall as usize
            });
            for idx in 0..fixed_operand_count {
                writer.byte(reg(&instruction.operands, idx)? as usize);
            }
            writer.byte(parameter_count);
            for idx in fixed_operand_count..instruction.operands.len() {
                writer.byte(reg(&instruction.operands, idx)? as usize);
            }
            Ok(())
        }
        "closure_new" => {
            if instruction.operands.len() < 2 {
                return Err(CompileError::new(
                    "'closure_new' expects at least 2 operands",
                    -1,
                ));
            }
            let capture_count = instruction.operands.len() - 2;
            if capture_count > candlevm::constants::MAX_CAPTURE_COUNT as usize {
                return Err(CompileError::new(
                    format!(
                        "'closure_new' accepts at most {} captures",
                        candlevm::constants::MAX_CAPTURE_COUNT
                    ),
                    instruction.line as i32,
                ));
            }

            writer.byte(Opcode::NewClosure as usize);
            writer.byte(reg(&instruction.operands, 0)? as usize);
            writer.byte(reg(&instruction.operands, 1)? as usize);
            writer.byte(capture_count);
            for idx in 2..instruction.operands.len() {
                writer.byte(reg(&instruction.operands, idx)? as usize);
            }
            Ok(())
        }
        "jmp" | "jmp_if" | "jmp_if_not" => {
            let count = if instruction.name == "jmp" { 1 } else { 2 };
            exact(instruction, count)?;
            let delta = integer(&instruction.operands, 0, 32)? as isize;
            let dest = current as isize + delta;
            if dest < 0 || dest as usize >= offsets.len() {
                return Err(CompileError::new(
                    format!("Jump target line is outside the function: {dest}"),
                    0,
                ));
            }
            let byte_delta = if measure {
                0
            } else {
                offsets[dest as usize] as isize - offsets[current + 1] as isize
            };
            if !(-128..=127).contains(&byte_delta) {
                return Err(CompileError::new(
                    format!("Jump byte offset {byte_delta} is outside the signed byte range"),
                    0,
                ));
            }
            writer.byte(match instruction.name.as_str() {
                "jmp" => Opcode::Jump as usize,
                "jmp_if" => Opcode::JumpIf as usize,
                _ => Opcode::JumpIfNot as usize,
            });
            writer.i32(byte_delta as i32);
            if count == 2 {
                writer.byte(reg(&instruction.operands, 1)? as usize);
            }
            Ok(())
        }
        _ => {
            let (op, ids) = op_info(&instruction.name).ok_or_else(|| CompileError {
                message: format!("Unknown instruction '{}'", instruction.name),
                line: instruction.line as i32,
                operand: -1,
            })?;
            exact(instruction, ids.len())?;
            regs(instruction, writer, op, ids)
        }
    }
}
