use okf_application::{BundleBinding, BundleRegistry};
use okf_core::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default)]
    bundles: Vec<BundleBinding>,
}

pub fn default_home() -> Result<PathBuf> {
    if let Some(value) = std::env::var_os("OKF_HOME") {
        let path = PathBuf::from(value);
        if !path.is_absolute() {
            return Err(Error::new("INVALID_CONFIG", "OKF_HOME must be absolute", 2));
        }
        return Ok(path);
    }
    dirs::data_local_dir()
        .map(|p| p.join("okf-cli"))
        .ok_or_else(|| {
            Error::new(
                "INVALID_CONFIG",
                "cannot locate local data directory; set OKF_HOME",
                2,
            )
        })
}

pub struct ConfigRegistry {
    pub home: PathBuf,
    pub project: PathBuf,
    pub selected_scope: Option<String>,
}

impl ConfigRegistry {
    pub fn discover(home: PathBuf, cwd: PathBuf, selected_scope: Option<String>) -> Self {
        let cwd = fs::canonicalize(&cwd).unwrap_or(cwd);
        let project = cwd
            .ancestors()
            .find(|p| p.join(".okf-cli/config.toml").is_file() || p.join(".git").exists())
            .unwrap_or(&cwd)
            .to_path_buf();
        Self {
            home,
            project,
            selected_scope,
        }
    }

    fn path(&self, scope: &str) -> Result<PathBuf> {
        match scope {
            "project" => Ok(self.project.join(".okf-cli/config.toml")),
            "user" => Ok(self.home.join("config.toml")),
            _ => Err(Error::new(
                "INVALID_SCOPE",
                "scope must be project or user",
                2,
            )),
        }
    }

    fn load(&self, scope: &str) -> Result<Config> {
        let path = self.path(scope)?;
        if !path.exists() {
            return Ok(Config::default());
        }
        let text = fs::read_to_string(path).map_err(Error::io)?;
        toml::from_str(&text).map_err(|e| Error::new("INVALID_CONFIG", e.to_string(), 2))
    }

    fn save(&self, path: &Path, config: &Config) -> Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| Error::new("INVALID_CONFIG", "config needs a parent directory", 2))?;
        fs::create_dir_all(parent).map_err(Error::io)?;
        let text = toml::to_string_pretty(config).map_err(Error::io)?;
        let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(Error::io)?;
        tmp.write_all(text.as_bytes()).map_err(Error::io)?;
        tmp.as_file().sync_all().map_err(Error::io)?;
        tmp.persist(path).map_err(|e| Error::io(e.error))?;
        Ok(())
    }

    fn lock(&self, path: &Path) -> Result<ConfigLock> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(Error::io)?;
        }
        let lock = path.with_extension("lock");
        fs::OpenOptions::new().write(true).create_new(true).open(&lock)
            .map_err(|e| Error::new("CONFIG_LOCKED", format!("cannot lock {}: {e}; remove a stale lock only after confirming no writer is active", lock.display()), 5))?;
        Ok(ConfigLock(lock))
    }
}

impl BundleRegistry for ConfigRegistry {
    fn list(&self) -> Result<Vec<BundleBinding>> {
        let mut bindings = vec![];
        for scope in ["project", "user"] {
            if self.selected_scope.as_deref().is_some_and(|s| s != scope) {
                continue;
            }
            for mut binding in self.load(scope)?.bundles {
                binding.scope = scope.into();
                let root = PathBuf::from(&binding.root);
                binding.root = if root.is_absolute() {
                    root
                } else {
                    let base = if scope == "project" {
                        &self.project
                    } else {
                        &self.home
                    };
                    base.join(root)
                }
                .to_string_lossy()
                .into_owned();
                bindings.push(binding);
            }
        }
        bindings.sort_by(|a, b| (&a.name, &a.scope).cmp(&(&b.name, &b.scope)));
        Ok(bindings)
    }

    fn add(&self, binding: &BundleBinding) -> Result<()> {
        let path = self.path(&binding.scope)?;
        let _lock = self.lock(&path)?;
        let mut config = self.load(&binding.scope)?;
        if config.bundles.iter().any(|s| s.name == binding.name) {
            return Err(Error::new(
                "BUNDLE_EXISTS",
                "bundle alias already exists in this scope",
                2,
            ));
        }
        let mut stored = binding.clone();
        if binding.scope == "project" {
            if let Ok(relative) = Path::new(&binding.root).strip_prefix(&self.project) {
                stored.root = if relative.as_os_str().is_empty() {
                    ".".into()
                } else {
                    relative.to_string_lossy().replace('\\', "/")
                };
            }
        }
        config.bundles.push(stored);
        self.save(&path, &config)
    }

    fn remove(&self, name: &str, scope: &str) -> Result<()> {
        let path = self.path(scope)?;
        let _lock = self.lock(&path)?;
        let mut config = self.load(scope)?;
        let before = config.bundles.len();
        config.bundles.retain(|s| s.name != name);
        if before == config.bundles.len() {
            return Err(Error::new(
                "BUNDLE_NOT_FOUND",
                "bundle not found in this scope",
                3,
            ));
        }
        self.save(&path, &config)
    }
}

struct ConfigLock(PathBuf);
impl Drop for ConfigLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
