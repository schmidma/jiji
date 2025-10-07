use std::fs;
use std::io::{BufReader, BufWriter};
use std::{fs::File, io, net::TcpStream};

use camino::{Utf8Path, Utf8PathBuf};
use color_eyre::eyre::{bail, Context as _, Result};
use ssh2::{Session, Sftp};
use tracing::debug;

use crate::configuration::SftpConfiguration;
use crate::hashing::Hash;
use crate::storage::Storage;

pub struct SftpStorage {
    location: Utf8PathBuf,
    sftp: Sftp,
}

impl SftpStorage {
    /// Establish an SSH session and return an authenticated SFTP handle.
    pub fn connect(configuration: &SftpConfiguration) -> Result<Self> {
        let address = format!(
            "{}:{}",
            configuration.host,
            configuration.port.unwrap_or(22)
        );
        debug!("connecting to SFTP host {address}");
        let tcp = TcpStream::connect(&address)
            .wrap_err_with(|| format!("failed to connect to {address}"))?;

        let mut session = Session::new().wrap_err("failed to create SSH session")?;
        session.set_tcp_stream(tcp);
        session.handshake().wrap_err("failed SSH handshake")?;

        if let Some(password) = &configuration.password {
            session
                .userauth_password(&configuration.username, password)
                .wrap_err("failed password authentication")?;
        } else {
            session
                .userauth_agent(&configuration.username)
                .wrap_err("failed agent authentication")?;
        }

        if !session.authenticated() {
            bail!(
                "authentication failed for user '{}'",
                configuration.username
            );
        }

        let sftp = session.sftp().wrap_err("failed to open SFTP session")?;
        Ok(Self {
            location: configuration.location.clone().into(),
            sftp,
        })
    }

    /// Ensure all directories leading up to `remote_path` exist.
    fn create_remote_dir_all(&self, remote_path: &Utf8Path) -> Result<()> {
        let mut current = Utf8PathBuf::new();
        for component in remote_path.components() {
            current.push(component);
            if current.as_str().is_empty() {
                continue;
            }
            if let Err(err) = self.sftp.stat(current.as_std_path()) {
                debug!("creating missing remote directory: {current}");
                self.sftp
                    .mkdir(current.as_std_path(), 0o755)
                    .wrap_err_with(|| {
                        format!("failed to create remote directory {current}: {err}")
                    })?;
            }
        }
        Ok(())
    }

    /// Build remote storage path for a given hash.
    fn remote_path_for(&self, hash: Hash) -> Utf8PathBuf {
        let hex = hash.to_hex();
        let (prefix, suffix) = hex.split_at(2);
        self.location.join(prefix).join(suffix)
    }
}

impl Storage for SftpStorage {
    fn store(&self, hash: Hash, local: impl AsRef<Utf8Path>) -> Result<()> {
        let local = local.as_ref();
        let remote_path = self.remote_path_for(hash);

        // Ensure remote directories exist
        if let Some(parent) = remote_path.parent() {
            self.create_remote_dir_all(parent)
                .wrap_err_with(|| format!("failed to prepare directories for {remote_path}"))?;
        }

        let src_file =
            File::open(local).wrap_err_with(|| format!("failed to open local file {local}"))?;
        let mut src = BufReader::with_capacity(128 * 1024, src_file); // 128 KiB buffer
        let dst_file = self
            .sftp
            .create(remote_path.as_std_path())
            .wrap_err_with(|| format!("failed to open remote file for writing: {remote_path}"))?;
        let mut dst = BufWriter::with_capacity(128 * 1024, dst_file); // 128 KiB buffer

        io::copy(&mut src, &mut dst)
            .wrap_err_with(|| format!("failed to copy data to remote {remote_path}"))?;
        Ok(())
    }

    fn retrieve(&self, hash: Hash, destination: impl AsRef<Utf8Path>) -> Result<()> {
        let destination = destination.as_ref();
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .wrap_err_with(|| format!("failed to create directories for {parent}"))?;
        }

        let remote_path = self.remote_path_for(hash);

        let src_file = self
            .sftp
            .open(remote_path.as_str())
            .wrap_err_with(|| format!("failed to open remote file {remote_path} for reading"))?;
        let mut src = BufReader::with_capacity(128 * 1024, src_file); // 128 KiB buffer
        let dst_file = File::create(destination)
            .wrap_err_with(|| format!("failed to create local file {destination}"))?;
        let mut dst = BufWriter::with_capacity(128 * 1024, dst_file); // 128 KiB buffer

        io::copy(&mut src, &mut dst).wrap_err_with(|| {
            format!("failed to copy remote {remote_path} to local {destination}")
        })?;
        Ok(())
    }
}
