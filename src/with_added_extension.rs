use camino::{Utf8Path, Utf8PathBuf};

/// A trait to create a new path with an added extension.
///
/// This trait returns a new [`Utf8PathBuf`] with the extension appended to the filename, leaving
/// the original path unmodified.
pub trait WithAddedExtension {
    /// Returns a new path with the given extension appended to the filename.
    ///
    /// # Parameters
    ///
    /// - `extension`: The extension to add. See [`AddExtension::add_extension`] for details.
    ///
    /// # Examples
    ///
    /// ```
    /// use camino::{Utf8Path, Utf8PathBuf};
    ///
    /// let path = Utf8Path::new("file");
    /// let new_path = path.with_added_extension("txt");
    /// assert_eq!(new_path.as_str(), "file.txt");
    /// ```
    fn with_added_extension<S: AsRef<str>>(&self, extension: S) -> Utf8PathBuf;
}

impl<P> WithAddedExtension for P
where
    P: AsRef<Utf8Path>,
{
    fn with_added_extension<S: AsRef<str>>(&self, extension: S) -> Utf8PathBuf {
        Utf8PathBuf::from_path_buf(
            self.as_ref()
                .as_std_path()
                .with_added_extension(extension.as_ref()),
        )
        .expect("path is valid UTF-8")
    }
}
