use super::{CompileError, Writer};
use candlevm::constants::{
    BYTECODE_MAGIC, CANDLE_VERSION, MAX_CAPTURE_COUNT, MAX_FUNCTION_PARAMETERS,
};
use candlevm::opcode::Opcode;
use candlevm::value::Value;
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct Module {
    pub name: String,
    pub functions: Vec<super::CompiledFunction>,
}

#[derive(Debug, Clone)]
pub struct Plugin {
    pub name: String,
    pub version: String,
}

pub fn link(
    main: &str,
    modules: &[Module],
    plugins: &[Plugin],
    resources: &[(String, Vec<u8>)],
) -> Result<Vec<u8>, CompileError> {
    if modules.is_empty() || modules.len() > 32768 {
        return Err(CompileError::new(
            "Module count must be between 1 and 32768",
            -1,
        ));
    }

    if plugins.len() > 65535 {
        return Err(CompileError::new(
            "Plugin dependency count exceeds 65535",
            -1,
        ));
    }

    if resources.len() > 65535 {
        return Err(CompileError::new("Resource count exceeds 65535", -1));
    }

    for (name, data) in resources {
        if data.len() > 64 * 1024 * 1024 {
            return Err(CompileError::new(
                format!("Resource '{name}' exceeds 67108864 bytes"),
                -1,
            ));
        }
    }

    let mut plugin_names = HashSet::new();
    for plugin in plugins {
        if !plugin_names.insert(plugin.name.clone()) {
            return Err(CompileError::new(
                format!("Duplicate plugin dependency '{}'", plugin.name),
                -1,
            ));
        }
    }

    let mut names = HashSet::new();
    let mut found = false;
    for m in modules {
        if !names.insert(m.name.clone()) {
            return Err(CompileError::new(
                format!("Duplicate module name '{}'", m.name),
                -1,
            ));
        }

        if m.name == main {
            found = true;
        }

        if m.functions.len() > 65535 {
            return Err(CompileError::new(
                format!("Too many functions in module '{}'", m.name),
                -1,
            ));
        }

        let mut fs = HashSet::new();
        for f in &m.functions {
            if f.params > MAX_FUNCTION_PARAMETERS {
                return Err(CompileError::new(
                    format!("Function has an invalid parameter count: {}", f.name),
                    -1,
                ));
            }
            if f.captures > MAX_CAPTURE_COUNT {
                return Err(CompileError::new(
                    format!("Function has an invalid capture count: {}", f.name),
                    -1,
                ));
            }
            if !fs.insert(f.name.clone()) {
                return Err(CompileError::new(
                    format!("Duplicate function: {}::{}", m.name, f.name),
                    -1,
                ));
            }
        }
    }

    if !found {
        return Err(CompileError::new(
            format!("Main module was not found: {main}"),
            -1,
        ));
    }

    let m = modules
        .iter()
        .find(|m| m.name == main)
        .ok_or_else(|| CompileError::new(format!("Main module was not found: {main}"), -1))?;

    let entry = m
        .functions
        .iter()
        .find(|f| f.name == "main")
        .ok_or_else(|| CompileError::new("Main module must contain a 'main' function", -1))?;

    if entry.params != 0 || entry.captures != 0 {
        return Err(CompileError::new(
            "The main function must not have parameters or captures",
            -1,
        ));
    }

    let mut bytecode = bootstrap(main);
    let mut meta = Writer::default();
    meta.byte(plugins.len());
    meta.byte(plugins.len() >> 8);

    for p in plugins {
        meta.string(&p.name);
        meta.string(&p.version);
    }

    meta.byte(modules.len());
    meta.byte(modules.len() >> 8);

    for m in modules {
        meta.string(&m.name);
        meta.byte(m.functions.len());
        meta.byte(m.functions.len() >> 8);

        for f in &m.functions {
            meta.string(&f.name);
            meta.i32(bytecode.len() as i32);
            meta.byte(f.params as usize);
            meta.byte(f.captures as usize);
            bytecode.extend_from_slice(&f.bytecode);
        }
    }

    meta.byte(resources.len());
    meta.byte(resources.len() >> 8);

    for (name, data) in resources {
        meta.string(name);
        meta.u32(data.len() as u32);
        meta.bytes(data);
    }

    let mut result = BYTECODE_MAGIC.to_vec();
    result.extend(CANDLE_VERSION.to_le_bytes());
    result.extend(meta.data);
    result.extend(bytecode);
    Ok(result)
}

fn bootstrap(main: &str) -> Vec<u8> {
    let mut writer = Writer::default();
    writer.byte(Opcode::LoadModule as usize);
    writer.byte(0);
    writer.string(main);

    writer.byte(Opcode::LoadFunction as usize);
    writer.byte(1);
    writer.byte(0);
    writer.string("main");

    writer.byte(Opcode::Call as usize);
    writer.byte(255);
    writer.byte(1);
    writer.byte(0);

    writer.byte(Opcode::LoadConst as usize);
    writer.byte(2);
    Value::Byte(0).serialize(&mut writer.data);

    writer.byte(Opcode::Exit as usize);
    writer.byte(2);

    writer.data
}
