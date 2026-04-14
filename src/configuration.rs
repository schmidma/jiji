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

use crate::JijiRepository;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageListEntry {
    pub name: String,
    pub kind: &'static str,
    pub uri: String,
    pub is_default: bool,
    pub details: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageListReport {
    pub default_storage: Option<String>,
    pub entries: Vec<StorageListEntry>,
}

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
                (
                    whoami::username().wrap_err("failed to determine current username")?,
                    None,
                )
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

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Path { .. } => "file",
            Self::Sftp(_) => "sftp",
        }
    }

    pub fn to_uri(&self) -> String {
        match self {
            Self::Path { location } => format!("file://{location}"),
            Self::Sftp(sftp) => {
                let mut uri = String::from("sftp://");
                uri.push_str(&sftp.username);
                if let Some(password) = &sftp.password {
                    uri.push(':');
                    uri.push_str(password);
                }
                uri.push('@');
                uri.push_str(&sftp.host);
                if let Some(port) = sftp.port {
                    uri.push(':');
                    uri.push_str(&port.to_string());
                }
                uri.push(':');
                uri.push_str(&sftp.location);
                uri
            }
        }
    }

    pub fn details(&self) -> Vec<(String, String)> {
        match self {
            Self::Path { location } => vec![("location".to_string(), location.to_string())],
            Self::Sftp(sftp) => {
                let mut fields = vec![
                    ("username".to_string(), sftp.username.clone()),
                    ("host".to_string(), sftp.host.clone()),
                ];
                if let Some(password) = &sftp.password {
                    fields.push(("password".to_string(), password.clone()));
                }
                if let Some(port) = sftp.port {
                    fields.push(("port".to_string(), port.to_string()));
                }
                fields.push(("location".to_string(), sftp.location.clone()));
                fields
            }
        }
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
    pub(crate) fn load_configuration_fresh(&self) -> Result<Configuration> {
        Configuration::load(self.workspace_root().join("config.toml"))
            .wrap_err("failed to load repository configuration")
    }

    pub(crate) fn save_configuration(&self, configuration: &Configuration) -> Result<()> {
        configuration
            .save(self.workspace_root().join("config.toml"))
            .wrap_err("failed to save repository configuration")
    }

    fn add_storage_to_configuration(
        configuration: &mut Configuration,
        name: &str,
        uri: &str,
    ) -> Result<()> {
        let storage = StorageConfiguration::from_uri(uri).wrap_err_with(|| {
            format!("failed to parse storage URI '{uri}' for storage '{name}'")
        })?;
        configuration.storages.insert(name.to_string(), storage);
        if configuration.default_storage.is_none() {
            configuration.default_storage = Some(name.to_string());
        }
        Ok(())
    }

    fn remove_storage_from_configuration(
        configuration: &mut Configuration,
        name: &str,
    ) -> Result<()> {
        if !configuration.storages.contains_key(name) {
            bail!("storage '{name}' not found in repository configuration");
        }
        configuration.storages.remove(name);
        if configuration.default_storage.as_deref() == Some(name) {
            configuration.default_storage = None;
        }
        Ok(())
    }

    fn set_default_storage_in_configuration(
        configuration: &mut Configuration,
        name: &str,
    ) -> Result<()> {
        if !configuration.storages.contains_key(name) {
            bail!("storage '{name}' not found in repository configuration");
        }
        configuration.default_storage = Some(name.to_string());
        Ok(())
    }

    fn storage_list_from_configuration(configuration: &Configuration) -> StorageListReport {
        let mut entries = configuration
            .storages
            .iter()
            .map(|(name, storage)| StorageListEntry {
                name: name.clone(),
                kind: storage.kind(),
                uri: storage.to_uri(),
                is_default: configuration.default_storage.as_deref() == Some(name.as_str()),
                details: storage.details(),
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| left.name.cmp(&right.name));

        StorageListReport {
            default_storage: configuration.default_storage.clone(),
            entries,
        }
    }

    pub(crate) fn require_default_storage_from_configuration(
        configuration: &Configuration,
    ) -> Result<String> {
        let name = configuration.default_storage.clone().ok_or_else(|| {
            color_eyre::eyre::eyre!(
                "no default storage is configured; set one with `jiji storage default <name>` or inspect configured storages with `jiji storage list`"
            )
        })?;

        configuration.storages.get(&name).ok_or_else(|| {
            color_eyre::eyre::eyre!(
                "default storage '{name}' was not found in repository configuration"
            )
        })?;

        Ok(name)
    }

    pub fn add_storage(&self, name: &str, uri: &str) -> Result<()> {
        self.with_write_lock("storage add", |repository| {
            let mut configuration = repository.load_configuration_fresh()?;
            Self::add_storage_to_configuration(&mut configuration, name, uri)?;
            repository.save_configuration(&configuration)
        })
    }

    pub fn remove_storage(&self, name: &str) -> Result<()> {
        self.with_write_lock("storage remove", |repository| {
            let mut configuration = repository.load_configuration_fresh()?;
            Self::remove_storage_from_configuration(&mut configuration, name)?;
            repository.save_configuration(&configuration)
        })
    }

    pub fn storage_list(&self) -> Result<StorageListReport> {
        self.with_read_lock("storage list", |repository| {
            let configuration = repository.load_configuration_fresh()?;
            Ok(Self::storage_list_from_configuration(&configuration))
        })
    }

    pub fn require_default_storage(&self) -> Result<String> {
        self.with_read_lock("push/fetch", |repository| {
            let configuration = repository.load_configuration_fresh()?;
            Self::require_default_storage_from_configuration(&configuration)
        })
    }

    pub fn set_default_storage(&self, name: &str) -> Result<()> {
        self.with_write_lock("storage default", |repository| {
            let mut configuration = repository.load_configuration_fresh()?;
            Self::set_default_storage_in_configuration(&mut configuration, name)?;
            repository.save_configuration(&configuration)
        })
    }
}

#[cfg(test)]
use crate::test_utils::setup_repo;

#[cfg(test)]
#[test]
fn configuration_roundtrips_file_and_sftp_storage() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let path = Utf8PathBuf::try_from(temp_dir.path().join("config.toml")).unwrap();

    let mut config = Configuration {
        default_storage: Some("local".to_string()),
        storages: HashMap::new(),
    };
    config.storages.insert(
        "local".to_string(),
        StorageConfiguration::Path {
            location: "/tmp/jiji-backup".into(),
        },
    );
    config.storages.insert(
        "remote".to_string(),
        StorageConfiguration::Sftp(SftpConfiguration {
            username: "alice".to_string(),
            password: Some("secret".to_string()),
            host: "example.com".to_string(),
            port: Some(2222),
            location: "/srv/jiji".to_string(),
        }),
    );

    config.save(&path)?;
    let loaded = Configuration::load(&path)?;

    assert_eq!(loaded.default_storage.as_deref(), Some("local"));

    match loaded.storages.get("local") {
        Some(StorageConfiguration::Path { location }) => {
            assert_eq!(location, &Utf8PathBuf::from("/tmp/jiji-backup"));
        }
        other => panic!("expected local path storage, got {other:?}"),
    }

    match loaded.storages.get("remote") {
        Some(StorageConfiguration::Sftp(sftp)) => {
            assert_eq!(sftp.username, "alice");
            assert_eq!(sftp.password.as_deref(), Some("secret"));
            assert_eq!(sftp.host, "example.com");
            assert_eq!(sftp.port, Some(2222));
            assert_eq!(sftp.location, "/srv/jiji");
        }
        other => panic!("expected remote sftp storage, got {other:?}"),
    }

    Ok(())
}

#[cfg(test)]
#[test]
fn storage_list_sorts_entries_and_marks_default() -> Result<()> {
    let (repo, _tmp, _guard) = setup_repo()?;
    repo.add_storage("zeta", "file:///tmp/zeta")?;
    repo.add_storage("alpha", "sftp://alice@example.com:/srv/alpha")?;
    repo.set_default_storage("alpha")?;

    let report = repo.storage_list()?;

    assert_eq!(report.default_storage.as_deref(), Some("alpha"));
    assert_eq!(report.entries.len(), 2);
    assert_eq!(report.entries[0].name, "alpha");
    assert!(report.entries[0].is_default);
    assert_eq!(report.entries[0].kind, "sftp");
    assert_eq!(report.entries[0].uri, "sftp://alice@example.com:/srv/alpha");
    assert_eq!(
        report.entries[0].details,
        vec![
            ("username".to_string(), "alice".to_string()),
            ("host".to_string(), "example.com".to_string()),
            ("location".to_string(), "/srv/alpha".to_string()),
        ]
    );
    assert_eq!(report.entries[1].name, "zeta");
    assert!(!report.entries[1].is_default);
    assert_eq!(report.entries[1].kind, "file");
    assert_eq!(report.entries[1].uri, "file:///tmp/zeta");
    assert_eq!(
        report.entries[1].details,
        vec![("location".to_string(), "/tmp/zeta".to_string())]
    );

    Ok(())
}

#[cfg(test)]
#[test]
fn storage_list_reloads_configuration_after_external_change() -> Result<()> {
    let (repo, _tmp, _guard) = setup_repo()?;
    let config_path = repo.workspace_root().join("config.toml");

    Configuration {
        default_storage: Some("external".to_string()),
        storages: HashMap::from([(
            "external".to_string(),
            StorageConfiguration::Path {
                location: "/tmp/external".into(),
            },
        )]),
    }
    .save(&config_path)?;

    let report = repo.storage_list()?;

    assert_eq!(report.default_storage.as_deref(), Some("external"));
    assert_eq!(report.entries.len(), 1);
    assert_eq!(report.entries[0].name, "external");
    assert!(report.entries[0].is_default);
    assert_eq!(report.entries[0].uri, "file:///tmp/external");

    Ok(())
}

#[cfg(test)]
#[test]
fn add_storage_reloads_configuration_before_write() -> Result<()> {
    let (repo, _tmp, _guard) = setup_repo()?;
    let config_path = repo.workspace_root().join("config.toml");

    Configuration {
        default_storage: Some("external".to_string()),
        storages: HashMap::from([(
            "external".to_string(),
            StorageConfiguration::Path {
                location: "/tmp/external".into(),
            },
        )]),
    }
    .save(&config_path)?;

    repo.add_storage("new", "file:///tmp/new")?;

    let stored = Configuration::load(&config_path)?;
    assert_eq!(stored.default_storage.as_deref(), Some("external"));
    assert!(stored.storages.contains_key("external"));
    assert!(stored.storages.contains_key("new"));

    Ok(())
}

#[cfg(test)]
#[test]
fn remove_storage_can_leave_no_default_selected() -> Result<()> {
    let (repo, _tmp, _guard) = setup_repo()?;
    repo.add_storage("local", "file:///tmp/local")?;
    repo.add_storage("backup", "file:///tmp/backup")?;

    repo.remove_storage("local")?;

    let report = repo.storage_list()?;
    assert_eq!(report.default_storage, None);
    assert_eq!(report.entries.len(), 1);
    assert_eq!(report.entries[0].name, "backup");
    assert!(!report.entries[0].is_default);

    Ok(())
}

#[cfg(test)]
#[test]
fn require_default_storage_errors_with_guidance() -> Result<()> {
    let (repo, _tmp, _guard) = setup_repo()?;

    let error = repo.require_default_storage().unwrap_err();
    let message = error.to_string();

    assert!(message.contains("no default storage is configured"));
    assert!(message.contains("jiji storage default <name>"));
    assert!(message.contains("jiji storage list"));

    Ok(())
}
