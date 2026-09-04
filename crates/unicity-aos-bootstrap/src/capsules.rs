//! Embedded Community Edition capsule-set validation.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;

use crate::UNICITY_CE_MANIFEST;

pub(crate) fn capsule_assets_from_manifest() -> io::Result<Vec<String>> {
    let manifest = UNICITY_CE_MANIFEST
        .parse::<toml::Value>()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let capsules = manifest
        .get("capsule")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "embedded distro has no capsules",
            )
        })?;
    let mut assets = Vec::with_capacity(capsules.len());
    for capsule in capsules {
        let package = capsule
            .get("name")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "embedded capsule has no name")
            })?;
        let source = capsule
            .get("source")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "embedded capsule has no source")
            })?;
        let relative = Path::new(source);
        let mut components = relative.components();
        if components.next() != Some(std::path::Component::Normal(OsStr::new("capsules")))
            || components
                .next()
                .and_then(|component| match component {
                    std::path::Component::Normal(name) => Some(name),
                    _ => None,
                })
                .is_none()
            || components.next().is_some()
            || relative.extension() != Some(OsStr::new("capsule"))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("embedded capsule source is not canonical: {source}"),
            ));
        }
        let asset = relative
            .file_name()
            .expect("validated capsule source has a filename")
            .to_str()
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "capsule asset is not UTF-8")
            })?
            .to_owned();
        if asset != format!("{package}.capsule") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("embedded capsule source does not match package {package}"),
            ));
        }
        if assets.contains(&asset) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("embedded distro selects duplicate capsule asset {asset}"),
            ));
        }
        assets.push(asset);
    }
    Ok(assets)
}
pub(crate) fn validate_capsule_dir(path: &Path, expected: &[String]) -> io::Result<PathBuf> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "AOS capsule directory is unavailable at {}: {error}",
                path.display()
            ),
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "AOS capsule directory must be a real directory: {}",
                path.display()
            ),
        ));
    }
    let canonical = path.canonicalize()?;
    let mut actual = Vec::new();
    for entry in fs::read_dir(&canonical)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "AOS capsule directory contains a non-regular entry: {}",
                    entry.path().display()
                ),
            ));
        }
        let name = entry.file_name().into_string().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "AOS capsule directory contains a non-UTF-8 entry",
            )
        })?;
        actual.push(name);
    }
    actual.sort();
    let mut expected = expected.to_vec();
    expected.sort();
    if actual != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "AOS capsule set differs from Community Edition; expected {}, found {}",
                expected.len(),
                actual.len()
            ),
        ));
    }
    Ok(canonical)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::aos_home::AosHome;
    use crate::test_fixtures::fixtures::*;
    use std::ffi::OsString;
    use std::io::ErrorKind;
    #[test]
    fn capsule_override_must_be_absolute_real_and_exact() {
        let root = temporary_home();
        let home = AosHome::from_root(&root);
        let valid = install_capsule_fixtures(&root);
        assert_eq!(
            home.capsule_dir_with(|name| {
                (name == "UNICITY_AOS_CAPSULE_DIR").then(|| valid.clone().into_os_string())
            })
            .expect("valid package-manager capsule directory"),
            valid.canonicalize().expect("canonical capsule directory")
        );
        for invalid in [OsString::new(), OsString::from("relative/capsules")] {
            assert_eq!(
                home.capsule_dir_with(|_| Some(invalid.clone()))
                    .expect_err("invalid capsule override must fail")
                    .kind(),
                ErrorKind::InvalidInput
            );
        }
        fs::write(valid.join("unexpected.capsule"), b"unexpected")
            .expect("write unexpected capsule");
        assert_eq!(
            home.capsule_dir_with(|_| Some(valid.clone().into_os_string()))
                .expect_err("non-exact capsule set must fail")
                .kind(),
            ErrorKind::InvalidInput
        );
        fs::remove_dir_all(root).expect("remove temporary product home");
    }
}
