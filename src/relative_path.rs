use std::{env::current_dir, io};

use camino::{Utf8Path, Utf8PathBuf};
use pathdiff::diff_utf8_paths;

fn relative_utf8<P>(path: P) -> io::Result<Utf8PathBuf>
where
    P: AsRef<Utf8Path>,
{
    let cwd = current_dir()?;
    let cwd_utf8 = Utf8PathBuf::try_from(cwd).map_err(camino::FromPathBufError::into_io_error)?;

    diff_utf8_paths(path, cwd_utf8).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "cannot express path relative to the current working directory",
        )
    })
}

/// Extension trait to convert a path to a relative UTF-8 path.
pub trait AsRelativePath {
    /// Converts the path to a relative UTF-8 path with respect to the current working directory.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - the current working directory cannot be determined,
    /// - the current working directory is not valid UTF-8, or
    /// - the provided path cannot be expressed relative to the current directory.
    fn as_relative_path(&self) -> io::Result<Utf8PathBuf>;
}

impl<P> AsRelativePath for P
where
    P: AsRef<Utf8Path>,
{
    fn as_relative_path(&self) -> io::Result<Utf8PathBuf> {
        relative_utf8(self)
    }
}
