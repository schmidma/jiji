use std::{
    collections::HashMap,
    fs::{self, read_to_string},
};

use camino::{Utf8Path, Utf8PathBuf};
use color_eyre::{
    eyre::{bail, Context as _, ContextCompat},
    Result,
};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::JijiRepository;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SftpConfiguration {
    pub username: String,
    pub password: Option<String>,
    pub host: String,
    pub port: Option<u16>,
    pub location: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum StorageConfiguration {
    Path { location: Utf8PathBuf },
    Sftp(SftpConfiguration),
}

impl StorageConfiguration {
    fn from_uri(uri: &str) -> Result<Self> {
        if let Some(stripped) = uri.strip_prefix("sftp://") {
            let (user_info, suffix) = stripped.split_once('@').unwrap_or(("", stripped));

            let (username, password) = if let Some((username, password)) = user_info.split_once(':')
            {
                (username.to_string(), Some(password))
            } else if !user_info.is_empty() {
                (user_info.to_string(), None)
            } else {
                (whoami::username(), None)
            };

            let (host_info, location) = suffix
                .rsplit_once(':')
                .wrap_err_with(|| format!("failed to parse SFTP URI: missing ':' in '{suffix}'"))?;

            let (host, port) = if let Some((host, port)) = host_info.split_once(':') {
                let port = port
                    .parse()
                    .wrap_err_with(|| format!("failed to parse port number from {port}"))?;
                (host, Some(port))
            } else {
                (host_info, None)
            };

            return Ok(StorageConfiguration::Sftp(SftpConfiguration {
                username,
                password: password.map(|s| s.to_string()),
                host: host.to_string(),
                port,
                location: location.to_string(),
            }));
        }
        if let Some(path) = uri.strip_prefix("file://") {
            return Ok(StorageConfiguration::Path {
                location: path.into(),
            });
        }
        bail!("unsupported storage URI scheme in '{uri}'")
    }
}

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct Configuration {
    pub default_storage: Option<String>,
    pub storages: HashMap<String, StorageConfiguration>,
}

impl Configuration {
    pub fn load(path: impl AsRef<Utf8Path>) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Configuration::default());
        }
        let data = read_to_string(path)
            .wrap_err_with(|| format!("failed to read configuration file at '{path}'"))?;
        let config: Configuration = toml::from_str(&data)
            .wrap_err_with(|| format!("failed to parse configuration file at '{path}'"))?;
        Ok(config)
    }

    pub fn save(&self, path: impl AsRef<Utf8Path>) -> Result<()> {
        let path = path.as_ref();
        let data =
            toml::to_string_pretty(self).wrap_err("failed to serialize configuration to TOML")?;
        fs::write(path, data)
            .wrap_err_with(|| format!("failed to write configuration file at '{path}'"))?;
        Ok(())
    }
}

impl JijiRepository {
    pub fn add_storage(&mut self, name: &str, uri: &str) -> Result<()> {
        let storage = StorageConfiguration::from_uri(uri).wrap_err_with(|| {
            format!("failed to parse storage URI '{uri}' for storage '{name}'")
        })?;
        self.configuration
            .storages
            .insert(name.to_string(), storage);
        if self.configuration.default_storage.is_none() {
            self.configuration.default_storage = Some(name.to_string());
        }
        let path = self.workspace_root().join("config.toml");
        self.configuration.save(path)
    }

    pub fn remove_storage(&mut self, name: &str) -> Result<()> {
        if !self.configuration.storages.contains_key(name) {
            bail!("storage '{name}' not found in repository configuration");
        }
        self.configuration.storages.remove(name);
        if self.configuration.default_storage.as_deref() == Some(name) {
            self.configuration.default_storage = None;
            warn!("removed storage was the default storage; no default storage is set now");
        }
        let path = self.workspace_root().join("config.toml");
        self.configuration.save(path)
    }

    pub fn set_default_storage(&mut self, name: &str) -> Result<()> {
        if !self.configuration.storages.contains_key(name) {
            bail!("storage '{name}' not found in repository configuration");
        }
        self.configuration.default_storage = Some(name.to_string());
        let path = self.workspace_root().join("config.toml");
        self.configuration.save(path)
    }
}
