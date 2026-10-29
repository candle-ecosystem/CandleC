use crate::{
    compiler::{self, Module},
    config::Config,
    diagnostic::{self, Reporter},
    lexer, parser,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    path::{Component, Path, PathBuf},
};

fn sources(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    fn walk(p: &Path, v: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for e in fs::read_dir(p)? {
            let p = e?.path();
            let kind = fs::symlink_metadata(&p)?.file_type();
            if kind.is_dir() {
                walk(&p, v)?;
            } else if kind.is_file() && p.extension().is_some_and(|x| x == "casm") {
                v.push(p);
            }
        }
        Ok(())
    }

    walk(root, &mut out).map_err(|e| e.to_string())?;
    out.sort();
    Ok(out)
}

fn source_module_name(source_root: &Path, source_file: &Path) -> Result<String, String> {
    let relative = source_file.strip_prefix(source_root).map_err(|_| {
        format!(
            "Source file is outside the configured source directory: {}",
            source_file.display()
        )
    })?;
    let relative = relative.with_extension("");
    let components = relative
        .components()
        .map(|component| match component {
            Component::Normal(name) => {
                let name = name.to_string_lossy();
                if name.contains('.') {
                    Err(format!(
                        "Source path component '{name}' contains '.', which is reserved as the module namespace separator: {}",
                        source_file.display()
                    ))
                } else {
                    Ok(name.into_owned())
                }
            }
            _ => Err(format!(
                "Source file does not have a relative module path: {}",
                source_file.display()
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;

    if components.is_empty() {
        return Err(format!(
            "Source file does not have a module name: {}",
            source_file.display()
        ));
    }

    Ok(components.join("."))
}

fn compiler_error_column(program: &parser::Program, line: i32, operand: i32) -> usize {
    let line = line.max(0) as usize;
    let Some(instruction) = program
        .functions
        .iter()
        .flat_map(|function| &function.instructions)
        .find(|instruction| instruction.line == line)
    else {
        return 0;
    };
    if operand >= 0 {
        instruction
            .operand_columns
            .get(operand as usize)
            .copied()
            .unwrap_or(instruction.column)
    } else {
        instruction.column
    }
}

fn resource_paths(root: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let mut out = Vec::new();
    if !root.is_dir() {
        return Ok(out);
    }
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, PathBuf)>) -> std::io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            if fs::symlink_metadata(&path)?.file_type().is_dir() {
                walk(&path, root, out)?;
            } else {
                let name = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((name, path));
            }
        }
        Ok(())
    }
    walk(root, root, &mut out).map_err(|e| e.to_string())?;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

type Key = (String, String);

struct Reachability {
    functions: HashSet<Key>,
    modules: HashSet<String>,
    resources: HashSet<String>,
    signatures: Vec<SignatureCheck>,
}

struct SignatureCheck {
    module: String,
    function: String,
    index: usize,
    expected: usize,
    actual: usize,
    target: String,
    closure: bool,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum AbstractValue {
    Module(String),
    Function(String, String),
    Unknown,
}

fn register(o: &parser::Operand) -> Option<u8> {
    match o {
        parser::Operand::Register(r) if *r != 255 => Some(*r),
        parser::Operand::SelfLit => Some(0),
        _ => None,
    }
}

type State = HashMap<u8, HashSet<AbstractValue>>;

fn propagate(
    target: usize,
    state: &State,
    count: usize,
    incoming: &mut HashMap<usize, State>,
    pending: &mut VecDeque<usize>,
) {
    if target >= count {
        return;
    }

    let Some(existing) = incoming.get_mut(&target) else {
        incoming.insert(target, state.clone());
        pending.push_back(target);
        return;
    };

    let mut changed = false;
    let ids: HashSet<u8> = existing.keys().chain(state.keys()).copied().collect();
    for id in ids {
        let old = existing.get_mut(&id);
        let next = state.get(&id);
        match (old, next) {
            (None, Some(_)) => {
                existing.insert(id, HashSet::from([AbstractValue::Unknown]));
                changed = true;
            }
            (Some(values), None) => changed |= values.insert(AbstractValue::Unknown),
            (Some(values), Some(next_values)) => {
                let before = values.len();
                values.extend(next_values.iter().cloned());
                changed |= values.len() != before;
            }
            (None, None) => {}
        }
    }

    if changed {
        pending.push_back(target);
    }
}

fn jump_delta(o: &[parser::Operand]) -> Option<isize> {
    let parser::Operand::Number(s) = o.first()? else {
        return None;
    };

    if let Ok(v) = s.parse::<i32>() {
        return Some(v as isize);
    }

    let (neg, h) = s
        .strip_prefix('-')
        .map_or((false, s.as_str()), |v| (true, v));

    let h = h.strip_prefix("0x").or_else(|| h.strip_prefix("0X"))?;
    let n = i32::from_str_radix(h, 16).ok()?;
    if neg {
        Some(-(n as isize))
    } else {
        Some(n as isize)
    }
}

fn analyze(
    main: &str,
    programs: &HashMap<String, parser::Program>,
) -> Result<Reachability, String> {
    if !programs.contains_key(main) {
        return Err(format!("Main module was not found: {main}"));
    }

    let mut all = HashMap::<Key, &parser::Function>::new();
    for (module, program) in programs {
        for f in &program.functions {
            all.insert((module.clone(), f.name.clone()), f);
        }
    }

    let main_key = (main.to_string(), "main".to_string());
    if !all.contains_key(&main_key) {
        return Err("Main module must contain a 'main' function".into());
    }

    let mut functions = HashSet::new();
    let mut modules = HashSet::from([main.to_string()]);
    let mut resources = HashSet::new();
    let mut pending_functions = VecDeque::from([main_key]);
    let mut analyzed = HashSet::new();
    let mut signatures = Vec::new();
    while let Some(key) = pending_functions.pop_front() {
        if !analyzed.insert(key.clone()) {
            continue;
        }
        functions.insert(key.clone());

        let Some(function) = all.get(&key) else {
            continue;
        };

        let count = function.instructions.len();
        if count == 0 {
            continue;
        }

        let mut initial = State::new();
        initial.insert(0, HashSet::from([AbstractValue::Module(key.0.clone())]));
        let mut incoming = HashMap::from([(0usize, initial)]);
        let mut pending = VecDeque::from([0usize]);
        let mut unknown_call = false;
        while let Some(index) = pending.pop_front() {
            if index >= count {
                continue;
            }

            let instruction = &function.instructions[index];
            let o = &instruction.operands;
            let mut regs = incoming.get(&index).cloned().unwrap_or_default();
            match instruction.name.as_str() {
                "load_mod" if o.len() >= 2 => {
                    if let (Some(dst), parser::Operand::String(m)) = (register(&o[0]), &o[1]) {
                        modules.insert(m.clone());
                        regs.insert(dst, HashSet::from([AbstractValue::Module(m.clone())]));
                    }
                }
                "load_res" if o.len() >= 2 => {
                    if let parser::Operand::String(r) = &o[1] {
                        resources.insert(r.clone());
                    }
                    if let Some(dst) = o.first().and_then(register) {
                        regs.insert(dst, HashSet::from([AbstractValue::Unknown]));
                    }
                }
                "load_func" if o.len() >= 3 => {
                    if let (Some(dst), Some(mreg), parser::Operand::String(f)) =
                        (register(&o[0]), register(&o[1]), &o[2])
                    {
                        let mut targets = HashSet::new();
                        for v in regs.get(&mreg).into_iter().flatten() {
                            if let AbstractValue::Module(m) = v {
                                let k = (m.clone(), f.clone());
                                if all.contains_key(&k) {
                                    functions.insert(k.clone());
                                }
                                targets.insert(AbstractValue::Function(m.clone(), f.clone()));
                            } else {
                                targets.insert(AbstractValue::Unknown);
                            }
                        }
                        if targets.is_empty() {
                            targets.insert(AbstractValue::Unknown);
                        }
                        regs.insert(dst, targets);
                    }
                }
                "mov" if o.len() >= 2 => {
                    if let (Some(dst), Some(src)) = (register(&o[0]), register(&o[1])) {
                        regs.insert(
                            dst,
                            regs.get(&src)
                                .cloned()
                                .unwrap_or_else(|| HashSet::from([AbstractValue::Unknown])),
                        );
                    }
                }
                "closure_new" if o.len() >= 2 => {
                    if let (Some(dst), Some(function_reg)) = (register(&o[0]), register(&o[1])) {
                        regs.insert(
                            dst,
                            regs.get(&function_reg)
                                .cloned()
                                .unwrap_or_else(|| HashSet::from([AbstractValue::Unknown])),
                        );
                    }
                }
                "call" | "pcall" => {
                    let ti = if instruction.name == "call" { 1 } else { 2 };
                    let targets = o
                        .get(ti)
                        .and_then(register)
                        .and_then(|r| regs.get(&r))
                        .cloned()
                        .unwrap_or_else(|| HashSet::from([AbstractValue::Unknown]));
                    let mut known = false;
                    let mut unknown = false;
                    for t in targets {
                        if let AbstractValue::Function(m, f) = t {
                            let k = (m, f);
                            if all.contains_key(&k) {
                                pending_functions.push_back(k.clone());
                                functions.insert(k);
                            }
                            known = true;
                        } else {
                            unknown = true;
                        }
                    }
                    if !known || unknown {
                        unknown_call = true;
                    }
                    if let Some(dst) = o.first().and_then(register) {
                        regs.insert(dst, HashSet::from([AbstractValue::Unknown]));
                    }
                    if instruction.name == "pcall"
                        && let Some(dst) = o.get(1).and_then(register)
                    {
                        regs.insert(dst, HashSet::from([AbstractValue::Unknown]));
                    }
                }
                "store_global" | "ret" | "throw" | "exit" | "nop" | "jmp" | "jmp_if"
                | "jmp_if_not" => {}
                _ => {
                    if let Some(dst) = o.first().and_then(register) {
                        regs.insert(dst, HashSet::from([AbstractValue::Unknown]));
                    }
                }
            }

            regs.insert(0, HashSet::from([AbstractValue::Module(key.0.clone())]));
            match instruction.name.as_str() {
                "jmp" => {
                    if let Some(delta) = jump_delta(o) {
                        let target = index as isize + delta;
                        if target >= 0 {
                            propagate(target as usize, &regs, count, &mut incoming, &mut pending);
                        }
                    }
                }
                "jmp_if" | "jmp_if_not" => {
                    if let Some(delta) = jump_delta(o) {
                        let target = index as isize + delta;
                        if target >= 0 {
                            propagate(target as usize, &regs, count, &mut incoming, &mut pending);
                        }
                    }
                    propagate(index + 1, &regs, count, &mut incoming, &mut pending);
                }
                "ret" | "throw" | "exit" => {}
                _ => propagate(index + 1, &regs, count, &mut incoming, &mut pending),
            }
        }

        for (index, state) in &incoming {
            let instruction = &function.instructions[*index];
            let (target_index, fixed, closure) = match instruction.name.as_str() {
                "call" => (1, 2, false),
                "pcall" => (2, 3, false),
                "closure_new" => (1, 2, true),
                _ => continue,
            };
            if instruction.operands.len() < fixed {
                continue;
            }
            let Some(targets) = instruction
                .operands
                .get(target_index)
                .and_then(register)
                .and_then(|r| state.get(&r))
            else {
                continue;
            };

            if targets.len() != 1 {
                continue;
            }
            let Some(AbstractValue::Function(module, name)) = targets.iter().next() else {
                continue;
            };
            let Some(target) = all.get(&(module.clone(), name.clone())) else {
                continue;
            };
            let expected = if closure {
                target.captures
            } else {
                target.params
            } as usize;
            let actual = instruction.operands.len() - fixed;
            if actual != expected {
                signatures.push(SignatureCheck {
                    module: key.0.clone(),
                    function: key.1.clone(),
                    index: *index,
                    expected,
                    actual,
                    target: format!("{module}::{name}"),
                    closure,
                });
            }
        }
        if unknown_call {
            for k in all.keys() {
                pending_functions.push_back(k.clone());
                functions.insert(k.clone());
            }
        }
    }

    Ok(Reachability {
        functions,
        modules,
        resources,
        signatures,
    })
}

fn analysis_diagnostics(
    config: &Config,
    programs: &HashMap<String, parser::Program>,
    paths: &HashMap<String, PathBuf>,
    module_order: &[String],
    reachable: &Reachability,
    resources: &[(String, PathBuf)],
) -> Vec<diagnostic::Diagnostic> {
    let mut diagnostics = Vec::new();
    for name in module_order {
        let Some(program) = programs.get(name) else {
            continue;
        };
        let path = paths.get(name).map(|p| p.display().to_string());
        for function in &program.functions {
            let key = (name.clone(), function.name.clone());
            if (!config.dce || reachable.functions.contains(&key))
                && function.instructions.last().is_none_or(|i| i.name != "ret")
            {
                diagnostics.push(diagnostic::Diagnostic::warning(
                    "W_MISSING_RET",
                    format!(
                        "function has no 'ret' instruction: '{}::{}'; inserted 'ret void'",
                        name, function.name
                    ),
                    path.clone(),
                    function.line,
                    function.column,
                ));
            }
        }
        if !reachable.modules.contains(name) {
            diagnostics.push(diagnostic::Diagnostic::warning(
                "W_UNUSED_MODULE",
                format!(
                    "Unused module: {}{}",
                    name,
                    if config.dce {
                        "; will be omitted from final executable"
                    } else {
                        ""
                    }
                ),
                path.clone(),
                program.functions.first().map_or(0, |f| f.line),
                program.functions.first().map_or(0, |f| f.column),
            ));
        } else {
            for function in &program.functions {
                if !reachable
                    .functions
                    .contains(&(name.clone(), function.name.clone()))
                {
                    diagnostics.push(diagnostic::Diagnostic::warning(
                        "W_UNUSED_FUNCTION",
                        format!(
                            "Unused function: {}::{}{}",
                            name,
                            function.name,
                            if config.dce {
                                "; will be omitted from final executable"
                            } else {
                                ""
                            }
                        ),
                        path.clone(),
                        function.line,
                        function.column,
                    ));
                }
            }
        }
    }
    for (name, path) in resources {
        if !reachable.resources.contains(name) {
            diagnostics.push(diagnostic::Diagnostic::warning(
                "W_UNUSED_RESOURCE",
                format!(
                    "Unused resource: {}{}",
                    path.display(),
                    if config.dre {
                        "; will be omitted from final executable"
                    } else {
                        ""
                    }
                ),
                Some(path.display().to_string()),
                0,
                0,
            ));
        }
    }
    diagnostics
}

fn report_warnings(reporter: &mut Reporter, diagnostics: Vec<diagnostic::Diagnostic>) {
    for d in diagnostics {
        reporter.warning(
            d.code,
            d.message,
            d.file,
            d.line.unwrap_or(0),
            d.column.unwrap_or(0),
        );
    }
}

fn report_signatures(
    reporter: &mut Reporter,
    reachable: &Reachability,
    programs: &HashMap<String, parser::Program>,
    paths: &HashMap<String, PathBuf>,
) {
    let mut checks: Vec<_> = reachable.signatures.iter().collect();
    checks
        .sort_by(|a, b| (&a.module, &a.function, a.index).cmp(&(&b.module, &b.function, b.index)));
    for check in checks {
        let instruction = &programs[&check.module]
            .functions
            .iter()
            .find(|f| f.name == check.function)
            .unwrap()
            .instructions[check.index];
        let file = paths.get(&check.module).map(|p| p.display().to_string());
        if check.closure {
            reporter.error(
                "E_CAPTURE_COUNT",
                format!(
                    "'{}' expects exactly {} captures, but {} were supplied",
                    check.target, check.expected, check.actual
                ),
                file,
                instruction.line,
                instruction.column,
            );
        } else if check.actual > check.expected {
            reporter.warning(
                "W_EXTRA_ARGUMENTS",
                format!(
                    "'{}' expects {} parameters, but {} were supplied; extra parameters will be ignored",
                    check.target, check.expected, check.actual
                ),
                file,
                instruction.line,
                instruction.column,
            );
        } else {
            reporter.warning("W_MISSING_ARGUMENTS", format!("'{}' expects {} parameters, but {} were supplied; missing parameters will be filled with void", check.target, check.expected, check.actual), file, instruction.line, instruction.column);
        }
    }
}

pub fn build(root: &Path, reporter: &mut Reporter) -> Result<PathBuf, String> {
    let c = Config::read(root)?;
    if !c.source.is_dir() {
        return Err(format!(
            "Source directory not found: {}",
            c.source.display()
        ));
    }

    let files = sources(&c.source)?;
    if files.is_empty() {
        return Err(format!(
            "No .casm source files found in {}",
            c.source.display()
        ));
    }

    reporter.info(&format!(
        "Compiling project... ({} source file{})",
        files.len(),
        if files.len() == 1 { "" } else { "s" }
    ));

    let mut modules = Vec::new();
    let mut programs = HashMap::new();
    let mut paths = HashMap::new();
    for file in files {
        let source = fs::read_to_string(&file).map_err(|e| e.to_string())?;
        let module = source_module_name(&c.source, &file)?;
        let tokens = match lexer::tokenize(&source) {
            Ok(tokens) => tokens,
            Err(e) => {
                reporter.error(
                    "E_LEX",
                    &e.message,
                    Some(file.display().to_string()),
                    e.line,
                    e.column,
                );
                return Err(diagnostic::ALREADY_REPORTED.into());
            }
        };
        let program = match parser::parse(&tokens) {
            Ok(program) => program,
            Err(e) => {
                reporter.error(
                    "E_PARSE",
                    &e.message,
                    Some(file.display().to_string()),
                    e.line,
                    e.column,
                );
                return Err(diagnostic::ALREADY_REPORTED.into());
            }
        };
        let functions = match compiler::compile(&program) {
            Ok(functions) => functions,
            Err(e) => {
                reporter.error(
                    "E_INSTRUCTION",
                    &e.message,
                    Some(file.display().to_string()),
                    e.line.max(0) as usize,
                    compiler_error_column(&program, e.line, e.operand),
                );
                return Err(diagnostic::ALREADY_REPORTED.into());
            }
        };
        paths.insert(module.clone(), file);
        programs.insert(module.clone(), program);
        modules.push(Module {
            name: module,
            functions,
        });
    }

    let reachable = analyze(&c.main_module, &programs)?;
    report_signatures(reporter, &reachable, &programs, &paths);
    if reporter.has_errors() {
        return Err(diagnostic::ALREADY_REPORTED.into());
    }

    for check in &reachable.signatures {
        if !check.closure {
            let program = programs.get_mut(&check.module).unwrap();
            let function = program
                .functions
                .iter_mut()
                .find(|f| f.name == check.function)
                .unwrap();
            let instruction = &mut function.instructions[check.index];
            if check.actual < check.expected {
                instruction.operands.extend(std::iter::repeat_n(
                    parser::Operand::Void,
                    check.expected - check.actual,
                ));
            } else {
                let fixed = if instruction.name == "pcall" { 3 } else { 2 };
                instruction.operands.truncate(fixed + check.expected);
            }
        }
    }
    for module in &mut modules {
        module.functions = compiler::compile(&programs[&module.name]).map_err(|e| e.to_string())?;
    }
    let reachable_functions = reachable.functions.clone();
    let reachable_modules = reachable.modules.clone();
    let reachable_resources = reachable.resources.clone();
    let resource_files = resource_paths(&c.resources)?;
    let module_order = modules.iter().map(|m| m.name.clone()).collect::<Vec<_>>();
    let analysis_diags = analysis_diagnostics(
        &c,
        &programs,
        &paths,
        &module_order,
        &reachable,
        &resource_files,
    );
    report_warnings(reporter, analysis_diags);

    if c.dce {
        modules.retain(|m| reachable_modules.contains(&m.name));
        for m in &mut modules {
            m.functions
                .retain(|f| reachable_functions.contains(&(m.name.clone(), f.name.clone())));
        }
    }

    let mut resource_pairs = resource_files
        .into_iter()
        .map(|(name, path)| Ok((name, fs::read(path).map_err(|e| e.to_string())?)))
        .collect::<Result<Vec<_>, String>>()?;

    if c.dre {
        resource_pairs.retain(|(name, _)| reachable_resources.contains(name));
    }

    let bytes = match compiler::link(&c.main_module, &modules, &c.plugins, &resource_pairs) {
        Ok(bytes) => bytes,
        Err(e) => {
            reporter.error("E_LINK", e.message, None, 0, 0);
            return Err(diagnostic::ALREADY_REPORTED.into());
        }
    };
    fs::create_dir_all(&c.output).map_err(|e| e.to_string())?;
    let output = c.output_file();
    fs::write(&output, bytes).map_err(|e| e.to_string())?;

    reporter.success(&format!("Done! ({})", output.display()));
    Ok(output)
}

pub fn check(root: &Path, reporter: &mut Reporter) -> Result<(), String> {
    let c = Config::read(root)?;
    if !c.source.is_dir() {
        return Err(format!(
            "Source directory not found: {}",
            c.source.display()
        ));
    }

    let files = sources(&c.source)?;
    if files.is_empty() {
        return Err(format!(
            "No .casm source files found in {}",
            c.source.display()
        ));
    }

    reporter.info(&format!(
        "Checking project... ({} source file{})",
        files.len(),
        if files.len() == 1 { "" } else { "s" }
    ));
    let mut programs = HashMap::new();
    let mut paths = HashMap::new();
    let mut module_order = Vec::new();
    let mut parse_failed = false;
    let mut resource_scan_failed = false;
    let mut uncertain_structure = false;
    let mut uncertain_functions = HashSet::<Key>::new();
    for file in files {
        let shown = file.display().to_string();
        let source = match fs::read_to_string(&file) {
            Ok(source) => source,
            Err(e) => {
                parse_failed = true;
                reporter.error("E_IO", e.to_string(), Some(shown), 0, 0);
                continue;
            }
        };
        let module = source_module_name(&c.source, &file)?;
        let tokens = match lexer::tokenize(&source) {
            Ok(tokens) => tokens,
            Err(e) => {
                parse_failed = true;
                reporter.error("E_LEX", e.message, Some(shown), e.line, e.column);
                continue;
            }
        };
        let program = match parser::parse(&tokens) {
            Ok(program) => program,
            Err(e) => {
                parse_failed = true;
                reporter.error("E_PARSE", e.message, Some(shown), e.line, e.column);
                continue;
            }
        };

        let mut function_names = HashSet::new();
        for function in &program.functions {
            if !function_names.insert(function.name.clone()) {
                uncertain_structure = true;
                reporter.error(
                    "E_DUPLICATE_FUNCTION",
                    format!(
                        "Duplicate function '{}' in module '{}'",
                        function.name, module
                    ),
                    Some(shown.clone()),
                    function.line,
                    function.column,
                );
            }
        }
        for error in compiler::validate(&program) {
            let matched = program.functions.iter().any(|function| {
                if function
                    .instructions
                    .iter()
                    .any(|instruction| instruction.line == error.line.max(0) as usize)
                {
                    uncertain_functions.insert((module.clone(), function.name.clone()));
                    true
                } else {
                    false
                }
            });
            if !matched {
                uncertain_structure = true;
            }
            reporter.error(
                "E_INSTRUCTION",
                error.message,
                Some(shown.clone()),
                error.line.max(0) as usize,
                compiler_error_column(&program, error.line, error.operand),
            );
        }
        if programs.contains_key(&module) {
            uncertain_structure = true;
            reporter.error(
                "E_DUPLICATE_MODULE",
                format!("Duplicate module name '{module}'"),
                Some(shown.clone()),
                program.functions.first().map_or(0, |f| f.line),
                program.functions.first().map_or(0, |f| f.column),
            );
            continue;
        }
        paths.insert(module.clone(), file);
        module_order.push(module.clone());
        programs.insert(module, program);
    }

    let resource_files = match resource_paths(&c.resources) {
        Ok(files) => files,
        Err(e) => {
            resource_scan_failed = true;
            reporter.error("E_IO", e, Some(c.resources.display().to_string()), 0, 0);
            Vec::new()
        }
    };
    let mut analysis_unavailable = false;
    let reachable = match analyze(&c.main_module, &programs) {
        Ok(reachable) => Some(reachable),
        Err(e) => {
            if !reporter.has_errors() {
                let location = programs
                    .get(&c.main_module)
                    .and_then(|p| p.functions.first())
                    .map(|f| (f.line, f.column))
                    .unwrap_or((0, 0));
                reporter.error(
                    "E_SEMANTIC",
                    e,
                    paths.get(&c.main_module).map(|p| p.display().to_string()),
                    location.0,
                    location.1,
                );
            }
            analysis_unavailable = true;
            None
        }
    };
    let fallback_reachability;
    let reachable = if let Some(reachable) = reachable {
        reachable
    } else {
        fallback_reachability = Reachability {
            functions: HashSet::new(),
            signatures: Vec::new(),
            modules: programs.keys().cloned().collect(),
            resources: resource_files
                .iter()
                .map(|(name, _)| name.clone())
                .collect(),
        };
        fallback_reachability
    };
    if !parse_failed
        && !uncertain_structure
        && uncertain_functions.is_empty()
        && !analysis_unavailable
    {
        report_signatures(reporter, &reachable, &programs, &paths);
    }
    let mut warnings = analysis_diagnostics(
        &c,
        &programs,
        &paths,
        &module_order,
        &reachable,
        &resource_files,
    );
    let invalid_reachable_function = uncertain_functions
        .iter()
        .any(|function| reachable.functions.contains(function));
    if parse_failed || uncertain_structure || invalid_reachable_function || analysis_unavailable {
        warnings.retain(|warning| warning.code == "W_MISSING_RET");
    } else if resource_scan_failed {
        warnings.retain(|warning| warning.code != "W_UNUSED_RESOURCE");
    }
    report_warnings(reporter, warnings);
    if let Some(main) = programs
        .get(&c.main_module)
        .and_then(|p| p.functions.iter().find(|f| f.name == "main"))
    {
        if main.params != 0 || main.captures != 0 {
            reporter.error(
                "E_SEMANTIC",
                "The main function must not have parameters or captures",
                paths.get(&c.main_module).map(|p| p.display().to_string()),
                main.line,
                if main.params != 0 {
                    main.params_column
                } else {
                    main.captures_column
                },
            );
        }
    }

    if reporter.has_errors() {
        return Err(diagnostic::ALREADY_REPORTED.into());
    }
    reporter.success("Source code is valid.");
    Ok(())
}

pub fn clean(root: &Path, reporter: &mut Reporter) -> Result<(), String> {
    let c = Config::read(root)?;
    let project = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let output = c.output.canonicalize().unwrap_or(c.output.clone());
    if fs::symlink_metadata(&c.output).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(format!(
            "The output directory is a symbolic link: {}",
            output.display()
        ));
    }

    if c.output == project || output == project {
        return Err("Refusing to clean the project root; configure an output subdirectory".into());
    }

    if !c.output.exists() {
        reporter.success("Done! (output directory is already empty)");
        return Ok(());
    }

    reporter.info(&format!(
        "Cleaning build directory... ({})",
        output.display()
    ));

    let mut entries = Vec::new();

    fn walk(p: &Path, v: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for e in fs::read_dir(p)? {
            let p = e?.path();
            if fs::symlink_metadata(&p)?.file_type().is_dir() {
                walk(&p, v)?;
            }
            v.push(p);
        }
        Ok(())
    }

    walk(&c.output, &mut entries).map_err(|e| e.to_string())?;

    entries.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    let n = entries.len();
    for p in entries {
        if p.is_dir() {
            fs::remove_dir(p).map_err(|e| e.to_string())?
        } else {
            fs::remove_file(p).map_err(|e| e.to_string())?
        }
    }

    reporter.success(&format!(
        "Done! ({} file{} removed)",
        n,
        if n == 1 { "" } else { "s" }
    ));
    Ok(())
}
