#[derive(Clone, Copy)]
pub enum Severity {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputMode {
    Human,
    Json,
}

pub struct Reporter {
    mode: OutputMode,
    diagnostics: Vec<Diagnostic>,
}

impl Reporter {
    pub fn new(mode: OutputMode) -> Self {
        Self {
            mode,
            diagnostics: Vec::new(),
        }
    }

    pub fn info(&self, message: &str) {
        if self.mode == OutputMode::Human {
            report(Severity::Info, message, None, 0, 0);
        }
    }

    pub fn success(&self, message: &str) {
        if self.mode == OutputMode::Human {
            report(Severity::Success, message, None, 0, 0);
        }
    }

    pub fn warning(
        &mut self,
        code: &'static str,
        message: impl Into<String>,
        file: Option<String>,
        line: usize,
        column: usize,
    ) {
        let diagnostic = Diagnostic::warning(code, message, file, line, column);
        self.emit(diagnostic, Severity::Warning);
    }

    pub fn error(
        &mut self,
        code: &'static str,
        message: impl Into<String>,
        file: Option<String>,
        line: usize,
        column: usize,
    ) {
        let diagnostic = Diagnostic::error(code, message, file, line, column);
        self.emit(diagnostic, Severity::Error);
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(|d| d.severity == "error")
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    fn emit(&mut self, diagnostic: Diagnostic, severity: Severity) {
        if self.mode == OutputMode::Human {
            report(
                severity,
                &diagnostic.message,
                diagnostic.file.as_deref(),
                diagnostic.line.unwrap_or(0),
                diagnostic.column.unwrap_or(0),
            );
        }
        self.diagnostics.push(diagnostic);
    }
}

pub const ALREADY_REPORTED: &str = "diagnostic already reported";

impl Severity {
    fn label(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Success => "success",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }

    fn color(self) -> &'static str {
        match self {
            Self::Success => "\x1b[32m",
            Self::Warning => "\x1b[33m",
            Self::Error => "\x1b[31m",
            Self::Info => "",
        }
    }
}

pub fn report(severity: Severity, message: &str, file: Option<&str>, line: usize, column: usize) {
    let label = severity.label();
    let color = severity.color();
    let reset = if color.is_empty() { "" } else { "\x1b[0m" };

    eprintln!("{color}{label}{reset}: {message}");
    if let Some(f) = file {
        if line > 0 {
            eprintln!(
                "  --> {f}:{line}{}",
                if column > 0 {
                    format!(":{column}")
                } else {
                    String::new()
                }
            );
        } else {
            eprintln!("  --> {f}");
        }
    }
}
#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub severity: &'static str,
    pub code: &'static str,
    pub message: String,
    pub file: Option<String>,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

impl Diagnostic {
    pub fn error(
        code: &'static str,
        message: impl Into<String>,
        file: Option<String>,
        line: usize,
        column: usize,
    ) -> Self {
        Self {
            severity: "error",
            code,
            message: message.into(),
            file,
            line: (line > 0).then_some(line),
            column: (column > 0).then_some(column),
        }
    }

    pub fn warning(
        code: &'static str,
        message: impl Into<String>,
        file: Option<String>,
        line: usize,
        column: usize,
    ) -> Self {
        Self {
            severity: "warning",
            code,
            message: message.into(),
            file,
            line: (line > 0).then_some(line),
            column: (column > 0).then_some(column),
        }
    }
}

fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c <= '\u{1f}' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn json_result(task: &str, success: bool, diagnostics: &[Diagnostic]) -> String {
    let errors = diagnostics.iter().filter(|d| d.severity == "error").count();
    let warnings = diagnostics
        .iter()
        .filter(|d| d.severity == "warning")
        .count();
    let items = diagnostics
        .iter()
        .map(|d| {
            let mut fields = vec![
                format!("\"severity\":{}", json_string(d.severity)),
                format!("\"code\":{}", json_string(d.code)),
                format!("\"message\":{}", json_string(&d.message)),
            ];
            if let Some(file) = &d.file {
                fields.push(format!("\"file\":{}", json_string(file)));
            }
            if let Some(line) = d.line {
                fields.push(format!("\"line\":{line}"));
            }
            if let Some(column) = d.column {
                fields.push(format!("\"column\":{column}"));
            }
            format!("{{{}}}", fields.join(","))
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"schema_version\":1,\"task\":{},\"success\":{},\"diagnostics\":[{}],\"summary\":{{\"errors\":{},\"warnings\":{}}}}}",
        json_string(task),
        success,
        items,
        errors,
        warnings
    )
}
