use color_eyre::Result;
use eyre::WrapErr;
use serde_yaml::Value;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(serde::Serialize, Debug, Clone)]
pub struct ServiceEntry {
    pub path: PathBuf,
    pub name: String,
    pub image: String,
    /// Full service config used for change detection; excluded from API output.
    #[serde(skip)]
    pub raw_config: Value,
}

/// Scans the filesystem starting from `root` and returns all services found
/// in YAML compose files. Non-compose YAML files are silently skipped.
pub fn scan_filesystem(root: &Path) -> Result<Vec<ServiceEntry>> {
    let mut results = Vec::new();
    for entry in WalkDir::new(root)
        .follow_links(true)
        .max_depth(10)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();

        tracing::debug!("Visiting path: {:?}", path);

        if path.extension().map(|e| e == "yaml" || e == "yml").unwrap_or(false) {
            tracing::debug!("Parsing YAML file: {:?}", path);
            match parse_yaml_file(path) {
                Ok(entries) => results.extend(entries),
                Err(e) => tracing::debug!("Skipping {path:?}: {e}"),
            }
        }
    }

    Ok(results)
}

/// Update the `image:` field of a single service in a compose file in-place.
///
/// Uses an atomic write (temp file → rename) so a failure never corrupts
/// the existing file. Returns an error if the service or its image field
/// is not found.
pub fn write_service_image(
    path: &Path,
    service_name: &str,
    new_image: &str,
) -> Result<()> {
    let content =
        fs::read_to_string(path).wrap_err_with(|| format!("Reading {path:?}"))?;

    let mut yaml: Value = serde_yaml::from_str(&content)
        .wrap_err_with(|| format!("Parsing {path:?}"))?;

    let svc =
        yaml.get_mut("services").and_then(|s| s.get_mut(service_name)).ok_or_else(
            || eyre::eyre!("Service '{service_name}' not found in {path:?}"),
        )?;

    let image_val = svc.get_mut("image").ok_or_else(|| {
        eyre::eyre!("No 'image' field for service '{service_name}' in {path:?}")
    })?;

    *image_val = Value::String(new_image.to_string());

    let new_content =
        serde_yaml::to_string(&yaml).wrap_err("Serializing updated compose file")?;

    let tmp = path.with_extension("tmp");
    fs::write(&tmp, new_content.as_bytes())
        .wrap_err_with(|| format!("Writing temp file {tmp:?}"))?;
    fs::rename(&tmp, path)
        .wrap_err_with(|| format!("Renaming {tmp:?} → {path:?}"))?;

    Ok(())
}

pub(crate) fn parse_yaml_file(path: &Path) -> Result<Vec<ServiceEntry>> {
    let content = fs::read_to_string(path)
        .wrap_err_with(|| format!("Reading file {path:?}"))?;
    parse_yaml_str(&content, path)
}

/// Parse compose services from an in-memory string. `path` is used only for
/// error messages and as the `ServiceEntry.path` value.
pub(crate) fn parse_yaml_str(
    content: &str,
    path: &Path,
) -> Result<Vec<ServiceEntry>> {
    let yaml: Value = serde_yaml::from_str(content)
        .wrap_err_with(|| format!("Parsing YAML file {path:?}"))?;

    let svcs = yaml
        .get("services")
        .and_then(|s| s.as_mapping())
        .ok_or_else(|| eyre::eyre!("No services found in {path:?}"))?;

    let mut entries = Vec::new();

    for (name, svc) in svcs {
        let service_name = name
            .as_str()
            .ok_or_else(|| eyre::eyre!("Invalid service name in {path:?}"))?;

        if let Some(image) =
            svc.get(Value::String("image".to_string())).and_then(Value::as_str)
        {
            entries.push(ServiceEntry {
                path: path.to_path_buf(),
                name: service_name.to_string(),
                image: image.to_string(),
                raw_config: svc.clone(),
            });
        }
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_scan_filesystem() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("docker-compose.yaml");
        let mut file = File::create(&file_path).unwrap();

        let yaml_content = r#"
services:
  web:
    image: nginx:latest
  db:
    image: postgres:alpine
"#;

        file.write_all(yaml_content.as_bytes()).unwrap();

        let results = scan_filesystem(dir.path()).unwrap();
        assert_eq!(results.len(), 2);

        let web_service = results.iter().find(|s| s.name == "web").unwrap();
        assert_eq!(web_service.image, "nginx:latest");

        let db_service = results.iter().find(|s| s.name == "db").unwrap();
        assert_eq!(db_service.image, "postgres:alpine");

        dir.close().unwrap();
    }

    #[test]
    fn test_parse_yaml_file_invalid() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("invalid.yaml");
        let mut file = File::create(&file_path).unwrap();

        let invalid_yaml_content = r#"
services:
  web
    image: nginx:latest
"#;

        file.write_all(invalid_yaml_content.as_bytes()).unwrap();

        let result = parse_yaml_file(&file_path);
        assert!(result.is_err());

        dir.close().unwrap();
    }

    #[test]
    fn test_parse_yaml_file_no_services() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("no_services.yaml");
        let mut file = File::create(&file_path).unwrap();

        let yaml_content = r#"
app:
  name: myapp
"#;

        file.write_all(yaml_content.as_bytes()).unwrap();

        let result = parse_yaml_file(&file_path);
        assert!(result.is_err());

        dir.close().unwrap();
    }

    #[test]
    fn test_parse_yaml_file_valid() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("valid.yaml");
        let mut file = File::create(&file_path).unwrap();

        let yaml_content = r#"
services:
  api:
    image: myapi:1.0
  cache:
    image: redis:latest
"#;

        file.write_all(yaml_content.as_bytes()).unwrap();

        let result = parse_yaml_file(&file_path).unwrap();
        assert_eq!(result.len(), 2);

        let api_service = result.iter().find(|s| s.name == "api").unwrap();
        assert_eq!(api_service.image, "myapi:1.0");

        let cache_service = result.iter().find(|s| s.name == "cache").unwrap();
        assert_eq!(cache_service.image, "redis:latest");

        dir.close().unwrap();
    }
}
