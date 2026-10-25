use crate::compiler::Plugin;
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct Config {
    pub name: String,
    pub version: String,
    pub main_module: String,
    pub source: PathBuf,
    pub resources: PathBuf,
    pub output: PathBuf,
    pub dce: bool,
    pub dre: bool,
    pub plugins: Vec<Plugin>,
}

fn get_string(t: &toml::Table, k: &str, d: &str) -> String {
    t.get(k)
        .and_then(toml::Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(d)
        .to_owned()
}

pub fn normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() && !path.is_absolute() {
                    normalized.push(component.as_os_str());
                }
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn resolve(root: &Path, s: String) -> PathBuf {
    let p = PathBuf::from(s);
    let joined = if p.is_absolute() { p } else { root.join(p) };
    normalize(&joined)
}

impl Config {
    pub fn read(root: &Path) -> Result<Self, String> {
        match fs::metadata(root) {
            Ok(metadata) if !metadata.is_dir() => {
                return Err(format!(
                    "Project path is not a directory: {}",
                    root.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(format!("Project directory not found: {}", root.display()));
            }
            Err(error) => {
                return Err(format!(
                    "Cannot access project directory '{}': {error}",
                    root.display()
                ));
            }
        }

        let config_path = root.join("candlec.toml");
        let txt = fs::read_to_string(&config_path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                format!(
                    "Project configuration file not found: {}",
                    config_path.display()
                )
            } else {
                format!(
                    "Cannot read project configuration file '{}': {error}",
                    config_path.display()
                )
            }
        })?;
        let value: toml::Value =
            toml::from_str(&txt).map_err(|e| format!("Invalid candlec.toml: {e}"))?;
        let empty = toml::Table::new();
        let project = value
            .get("project")
            .and_then(toml::Value::as_table)
            .unwrap_or(&empty);
        let paths = value
            .get("paths")
            .and_then(toml::Value::as_table)
            .unwrap_or(&empty);
        let name = get_string(
            project,
            "name",
            root.file_name()
                .and_then(|x| x.to_str())
                .unwrap_or("project"),
        );
        let version = get_string(project, "version", "0.1.0");
        for (k, v) in [("project.name", &name), ("project.version", &version)] {
            if v == "." || v == ".." || v.contains('/') || v.contains('\\') {
                return Err(format!(
                    "{k} must not contain path separators or be '.' or '..'"
                ));
            }
        }
        let opt = value
            .get("optimization")
            .map(|v| v.as_table().ok_or("optimization must be a TOML table"))
            .transpose()?;
        let flag = |key: &str, default| -> Result<bool, String> {
            match opt.and_then(|t| t.get(key)) {
                None => Ok(default),
                Some(toml::Value::Boolean(b)) => Ok(*b),
                _ => Err(format!("optimization.{key} must be a boolean")),
            }
        };
        let mut plugins = Vec::new();
        if let Some(tab) = value.get("plugins").and_then(toml::Value::as_table) {
            for (n, v) in tab {
                let vers = v
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| format!("Plugin '{n}' must have a version string"))?;
                plugins.push(Plugin {
                    name: n.clone(),
                    version: vers.into(),
                });
            }
        }
        Ok(Self {
            name,
            version,
            main_module: get_string(project, "main_module", "main"),
            source: resolve(root, get_string(paths, "source", "src")),
            resources: resolve(root, get_string(paths, "resources", "res")),
            output: resolve(root, get_string(paths, "output", "build")),
            dce: flag("dead_code_elimination", true)?,
            dre: flag("dead_resource_elimination", true)?,
            plugins,
        })
    }

    pub fn output_file(&self) -> PathBuf {
        self.output
            .join(format!("{}-{}.candle", self.name, self.version))
    }
}
