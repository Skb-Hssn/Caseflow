use crate::model::{Language, SourceSpec};
use std::fs;
use std::path::{Path, PathBuf};

pub fn resolve_source(requested: impl AsRef<Path>) -> Result<SourceSpec, String> {
    let requested = requested.as_ref().to_path_buf();
    let file_name = requested
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("invalid source path '{}'", requested.display()))?;

    let (path, language) = match Path::new(file_name)
        .extension()
        .and_then(|ext| ext.to_str())
    {
        Some(extension) => {
            let language = Language::from_extension(extension).ok_or_else(|| {
                format!(
                    "unsupported source extension in '{}'; use .cpp, .py, .c, .java, .rs, .go, or .kt",
                    requested.display()
                )
            })?;
            (requested.clone(), language)
        }
        None => (requested.with_extension("cpp"), Language::Cpp),
    };

    let stem = path.with_extension("");
    Ok(SourceSpec {
        requested,
        path,
        stem,
        language,
    })
}

pub fn scan_sources(directory: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("cannot read '{}': {error}", directory.display()))?;
    let mut sources = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            if !file_type.is_file() {
                return None;
            }
            let path = entry.path();
            let extension = path.extension()?.to_str()?;
            Language::from_extension(extension).map(|_| path)
        })
        .collect::<Vec<_>>();
    sources.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
    Ok(sources)
}

pub fn is_supported_source(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .and_then(Language::from_extension)
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensionless_source_is_cpp() {
        let source = resolve_source("round/a").unwrap();
        assert_eq!(source.path, PathBuf::from("round/a.cpp"));
        assert_eq!(source.stem, PathBuf::from("round/a"));
        assert_eq!(source.language, Language::Cpp);
    }

    #[test]
    fn known_extensions_resolve() {
        for language in Language::ALL {
            let source = resolve_source(format!("main.{}", language.extension())).unwrap();
            assert_eq!(source.language, language);
        }
    }

    #[test]
    fn unknown_extension_is_rejected() {
        assert!(resolve_source("main.txt").is_err());
    }
}
