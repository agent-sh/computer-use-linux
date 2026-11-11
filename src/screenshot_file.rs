//! Screenshot file output (`screenshot_file_path`): the encoded image is saved
//! on the host instead of returned as MCP image content, so a caller that does
//! not need to look at every capture spends no image tokens on it.
//!
//! The path is checked before anything is captured and again when the file is
//! written. The write goes to a private temp file in the same directory, which
//! is then renamed over the requested path: a reader never sees a partial
//! image, and an existing symlink at the path is never followed.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::screenshot_impl::ScreenshotOutputFormat;

/// The refusal for a call that reaches the server over `mcp --listen`.
pub(crate) const REMOTE_REFUSAL: &str = "screenshot_file_path is refused over HTTP (mcp --listen): a remote client must not write files on the server host. Omit it to get the image inline.";

/// Checks `path` for a `format` image and returns it as a path. `remote` is
/// set when the call came over HTTP, which never gets file output.
pub(crate) fn validate_screenshot_file_path(
    path: &str,
    format: ScreenshotOutputFormat,
    remote: bool,
) -> Result<PathBuf> {
    if remote {
        bail!(REMOTE_REFUSAL);
    }
    if path.is_empty() {
        bail!("screenshot_file_path is empty");
    }
    let path = Path::new(path);
    if !path.is_absolute() {
        bail!(
            "screenshot_file_path must be an absolute path, got {}",
            path.display()
        );
    }
    let Some(name) = path.file_name() else {
        bail!(
            "screenshot_file_path must name a file, got {}",
            path.display()
        );
    };
    let extension = Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("");
    let matches = match format {
        ScreenshotOutputFormat::Png => extension.eq_ignore_ascii_case("png"),
        ScreenshotOutputFormat::Jpeg => {
            extension.eq_ignore_ascii_case("jpg") || extension.eq_ignore_ascii_case("jpeg")
        }
    };
    if !matches {
        let expected = match format {
            ScreenshotOutputFormat::Png => ".png",
            ScreenshotOutputFormat::Jpeg => ".jpg or .jpeg",
        };
        bail!(
            "screenshot_file_path {} does not match format {}: use a {expected} extension, or set format to match the file",
            path.display(),
            format_name(format)
        );
    }
    let parent = path.parent().unwrap_or(Path::new("/"));
    match fs::metadata(parent) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => bail!(
            "the parent of screenshot_file_path, {}, is not a directory",
            parent.display()
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => bail!(
            "the parent directory of screenshot_file_path, {}, does not exist; it is not created",
            parent.display()
        ),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "cannot read the parent directory of screenshot_file_path, {}",
                    parent.display()
                )
            })
        }
    }
    check_existing_entry(path)?;
    Ok(path.to_path_buf())
}

/// An existing regular file may be replaced. A directory, a symlink, or any
/// other kind of entry is refused.
fn check_existing_entry(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            let kind = metadata.file_type();
            if kind.is_symlink() {
                bail!(
                    "screenshot_file_path {} is a symlink; pass the real path",
                    path.display()
                );
            }
            if kind.is_dir() {
                bail!("screenshot_file_path {} is a directory", path.display());
            }
            if !kind.is_file() {
                bail!(
                    "screenshot_file_path {} exists and is not a regular file",
                    path.display()
                );
            }
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("cannot inspect screenshot_file_path {}", path.display())),
    }
}

/// Writes `bytes` to `path` through a temp file in the same directory and a
/// rename. The file is created with owner-only permissions.
pub(crate) fn write_screenshot_file(path: &Path, bytes: &[u8]) -> Result<()> {
    check_existing_entry(path)?;
    let parent = path.parent().unwrap_or(Path::new("/"));
    let name = path
        .file_name()
        .context("screenshot_file_path must name a file")?
        .to_string_lossy();
    let suffix = getrandom::u64().context("cannot pick a temp file name")?;
    let temp = parent.join(format!(".{name}.{suffix:016x}.tmp"));
    let written = (|| -> io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.flush()?;
        drop(file);
        fs::rename(&temp, path)
    })();
    if let Err(error) = written {
        let _ = fs::remove_file(&temp);
        return Err(error)
            .with_context(|| format!("failed to write screenshot file {}", path.display()));
    }
    Ok(())
}

fn format_name(format: ScreenshotOutputFormat) -> &'static str {
    match format {
        ScreenshotOutputFormat::Png => "png",
        ScreenshotOutputFormat::Jpeg => "jpeg",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "cul-shot-file-{label}-{}-{:016x}",
                std::process::id(),
                getrandom::u64().unwrap()
            ));
            fs::create_dir(&dir).unwrap();
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn error(path: &str, format: ScreenshotOutputFormat, remote: bool) -> String {
        format!(
            "{:#}",
            validate_screenshot_file_path(path, format, remote).unwrap_err()
        )
    }

    #[test]
    fn refuses_bad_paths_before_capture() {
        let dir = TempDir::new("refuse");
        let png = ScreenshotOutputFormat::Png;
        let jpeg = ScreenshotOutputFormat::Jpeg;
        let ok = dir.0.join("shot.png");
        let ok = ok.to_str().unwrap();

        assert!(error(ok, png, true).contains("refused over HTTP"));
        assert!(error("", png, false).contains("is empty"));
        assert!(error("shot.png", png, false).contains("must be an absolute path"));
        assert!(error("/", png, false).contains("must name a file"));
        assert!(error(ok, jpeg, false).contains("does not match format jpeg"));
        let jpg = dir.0.join("shot.jpg");
        assert!(error(jpg.to_str().unwrap(), png, false).contains("does not match format png"));
        let bare = dir.0.join("shot");
        assert!(error(bare.to_str().unwrap(), png, false).contains("does not match format png"));

        let missing = dir.0.join("missing").join("shot.png");
        assert!(error(missing.to_str().unwrap(), png, false).contains("does not exist"));
        assert!(!dir.0.join("missing").exists(), "no directory is created");

        let file_parent = dir.0.join("file");
        fs::write(&file_parent, b"x").unwrap();
        let under_file = file_parent.join("shot.png");
        assert!(error(under_file.to_str().unwrap(), png, false).contains("is not a directory"));

        let as_dir = dir.0.join("dir.png");
        fs::create_dir(&as_dir).unwrap();
        assert!(error(as_dir.to_str().unwrap(), png, false).contains("is a directory"));

        let outside = dir.0.join("outside.png");
        fs::write(&outside, b"keep").unwrap();
        let link = dir.0.join("link.png");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        assert!(error(link.to_str().unwrap(), png, false).contains("is a symlink"));

        assert_eq!(
            validate_screenshot_file_path(ok, png, false).unwrap(),
            Path::new(ok)
        );
        for name in ["a.jpg", "a.JPEG", "a.jpeg"] {
            let path = dir.0.join(name);
            assert!(validate_screenshot_file_path(path.to_str().unwrap(), jpeg, false).is_ok());
        }
        assert!(
            validate_screenshot_file_path(dir.0.join("a.PNG").to_str().unwrap(), png, false)
                .is_ok()
        );
    }

    #[test]
    fn writes_atomically_and_replaces_a_regular_file() {
        let dir = TempDir::new("write");
        let path = dir.0.join("shot.png");
        write_screenshot_file(&path, b"first").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"first");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        write_screenshot_file(&path, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
        let entries: Vec<_> = fs::read_dir(&dir.0).unwrap().collect();
        assert_eq!(entries.len(), 1, "no temp file is left behind");

        // A symlink that appears after validation is still not followed.
        let outside = dir.0.join("outside.png");
        fs::write(&outside, b"keep").unwrap();
        let link = dir.0.join("link.png");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        assert!(write_screenshot_file(&link, b"new").is_err());
        assert_eq!(fs::read(&outside).unwrap(), b"keep");
    }

    #[test]
    fn a_failed_write_reports_and_leaves_no_temp_file() {
        // Root ignores directory permissions, so this needs an unprivileged user.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let dir = TempDir::new("fail");
        let locked = dir.0.join("locked");
        fs::create_dir(&locked).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o500)).unwrap();
        let path = locked.join("shot.png");
        let result = write_screenshot_file(&path, b"data");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
        let error = format!("{:#}", result.unwrap_err());
        assert!(error.contains("failed to write screenshot file"), "{error}");
        assert_eq!(fs::read_dir(&locked).unwrap().count(), 0);
    }
}
