use crate::{cli::InitArgs, error::Error};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    #[default]
    Desktop,
    Ssh,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
    pub backend: Backend,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    pub read_only: bool,
}
impl Profile {
    pub fn from_args(args: &InitArgs) -> Result<Self, Error> {
        let mut profile = Self {
            backend: args.backend,
            host: args.host.clone(),
            identity_file: args.identity_file.clone(),
            port: args.port,
            read_only: args.read_only,
        };
        profile.validate()?;
        if let Some(path) = &profile.identity_file {
            let expanded = if let Ok(rest) = path.strip_prefix("~") {
                dirs::home_dir()
                    .ok_or_else(|| failure("Cannot locate home directory"))?
                    .join(rest)
            } else {
                path.clone()
            };
            let absolute = expanded.canonicalize().map_err(|e| {
                failure(format!(
                    "Cannot access identity file {}: {e}",
                    path.display()
                ))
            })?;
            if !absolute.is_file() {
                return Err(failure("Identity file must be a file"));
            }
            profile.identity_file = Some(absolute);
        }
        Ok(profile)
    }
    pub fn validate(&self) -> Result<(), Error> {
        match self.backend {
            Backend::Desktop
                if self.host.is_some() || self.identity_file.is_some() || self.port.is_some() =>
            {
                return Err(failure(
                    "--host, --identity-file and --port require --backend ssh",
                ));
            }
            Backend::Ssh => {
                let host = self.host.as_deref().ok_or_else(|| {
                    failure("SSH profiles require --host HOST (an SSH alias or user@host)")
                })?;
                if host.is_empty()
                    || host.len() > 255
                    || host.starts_with('-')
                    || host.starts_with('@')
                    || host.ends_with('@')
                    || host.matches('@').count() > 1
                    || !host
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"@._-:[]".contains(&c))
                {
                    return Err(failure(
                        "Invalid SSH destination; use a host alias or user@host without whitespace or shell syntax",
                    ));
                }
                if self.port == Some(0) {
                    return Err(failure("SSH port must be between 1 and 65535"));
                }
            }
            _ => (),
        }
        Ok(())
    }
    pub fn target(&self) -> &str {
        self.host.as_deref().unwrap_or("local Windows desktop")
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub active_profile: Option<String>,
    pub profiles: BTreeMap<String, Profile>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            active_profile: None,
            profiles: BTreeMap::new(),
        }
    }
}
fn failure(message: impl Into<String>) -> Error {
    Error::new("config_error", message)
}
pub fn profile_name(name: &str) -> Result<String, String> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-.".contains(&c))
    {
        Err(
            "profile names must contain 1–64 ASCII letters, digits, dots, underscores or hyphens"
                .into(),
        )
    } else {
        Ok(name.into())
    }
}
pub fn path() -> Result<PathBuf, Error> {
    if let Some(path) = std::env::var_os("ONENOTE_CONFIG") {
        if path.is_empty() {
            return Err(failure("ONENOTE_CONFIG must not be empty"));
        }
        return Ok(PathBuf::from(path));
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            if cfg!(windows) {
                dirs::config_dir()
            } else {
                dirs::home_dir().map(|p| p.join(".config"))
            }
        });
    Ok(base
        .ok_or_else(|| failure("Cannot locate configuration directory; set ONENOTE_CONFIG"))?
        .join("onenote/config.toml"))
}
pub struct Store {
    pub path: PathBuf,
}
impl Store {
    pub fn open() -> Result<Self, Error> {
        Ok(Self { path: path()? })
    }
    pub fn read(&self) -> Result<Option<Config>, Error> {
        let file = match File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(failure(format!("Cannot read {}: {e}", self.path.display()))),
        };
        let mut contents = String::new();
        file.take(1024 * 1024 + 1)
            .read_to_string(&mut contents)
            .map_err(|e| failure(e.to_string()))?;
        if contents.len() > 1024 * 1024 {
            return Err(failure("Configuration exceeds 1 MiB"));
        }
        let config: Config = toml::from_str(&contents).map_err(|e| {
            failure(format!(
                "Invalid configuration in {}: {e}",
                self.path.display()
            ))
        })?;
        if config.version != 1 {
            return Err(failure(
                "Unsupported configuration version; expected version = 1",
            ));
        }
        for (name, profile) in &config.profiles {
            profile_name(name).map_err(failure)?;
            profile.validate()?;
        }
        if let Some(name) = &config.active_profile
            && !config.profiles.contains_key(name)
        {
            return Err(failure(format!(
                "Active profile '{name}' does not exist; repair {}",
                self.path.display()
            )));
        }
        Ok(Some(config))
    }
    pub fn resolve(&self, selected: Option<&str>) -> Result<(Option<String>, Profile), Error> {
        let config = self.read()?;
        let name = selected.or_else(|| config.as_ref().and_then(|c| c.active_profile.as_deref()));
        if let Some(name) = name {
            let profile = config
                .as_ref()
                .and_then(|c| c.profiles.get(name))
                .ok_or_else(|| {
                    failure(format!(
                        "Profile '{name}' does not exist; run `onenote init --profile {name}`"
                    ))
                })?;
            return Ok((Some(name.into()), profile.clone()));
        }
        if config.is_some() {
            return Err(failure(
                "No active profile; use `onenote profile use NAME` or `onenote init`",
            ));
        }
        Ok((None, Profile::default()))
    }
    pub fn update(
        &self,
        change: impl FnOnce(&mut Config) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        fs::create_dir_all(parent)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options.open(self.path.with_extension("toml.lock"))?;
        lock.try_lock().map_err(|_| {
            failure("Configuration is being updated by another process; retry shortly")
        })?;
        let mut config = self.read()?.unwrap_or_default();
        change(&mut config)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(
            toml::to_string_pretty(&config)
                .map_err(|e| failure(e.to_string()))?
                .as_bytes(),
        )?;
        file.as_file().sync_all()?;
        file.persist(&self.path)
            .map_err(|e| failure(format!("Cannot save {}: {e}", self.path.display())))?;
        Ok(())
    }
    pub fn ensure_available(&self, name: &str, force: bool) -> Result<(), Error> {
        if !force && self.read()?.is_some_and(|c| c.profiles.contains_key(name)) {
            return Err(failure(format!(
                "Profile '{name}' already exists; use --force to replace its settings"
            )));
        }
        Ok(())
    }
    pub fn save(&self, name: &str, profile: Profile, force: bool) -> Result<(), Error> {
        self.update(|config| {
            if config.profiles.contains_key(name) && !force {
                return Err(failure(format!(
                    "Profile '{name}' already exists; use --force to replace its settings"
                )));
            }
            config.profiles.insert(name.into(), profile);
            config.active_profile = Some(name.into());
            Ok(())
        })
    }
}
