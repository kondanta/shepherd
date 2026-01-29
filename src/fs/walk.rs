use color_eyre::Result;
use eyre::WrapErr;
use serde_yaml::Value;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(serde::Serialize, Debug)]
pub struct ServiceEntry {
    pub path: PathBuf,
    pub service: String,
    pub image: String,
}

// Scans the filesystem starting from `root` and returns a list of ServiceEntry found in YAML files.
// It looks for files with .yaml or .yml extensions and parses them to extract service information.
// We do not care about Dockerfiles here.
pub fn scan_filesystem(root: &Path) -> Result<Vec<ServiceEntry>> {
    let mut results = Vec::new();
    for entry in WalkDir::new(root)
        .follow_links(true)
        .max_depth(10) // Limit depth to avoid excessive recursion
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();

        tracing::debug!("Visiting path: {:?}", path);

        if path
            .extension()
            .map(|e| e == "yaml" || e == "yml")
            .unwrap_or(false)
        {
            tracing::debug!("Parsing YAML file: {:?}", path);
            results.extend(parse_yaml_file(path)?);
        }
    }

    Ok(results)
}

// Parses a YAML file at the given path and extracts service entries.
fn parse_yaml_file(path: &Path) -> Result<Vec<ServiceEntry>> {
    let content = fs::read_to_string(path)
        .wrap_err_with(|| format!("Rading file {path:?}"))?;

    let yaml: Value = serde_yaml::from_str(&content)
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

        if let Some(image) = svc
            .get(Value::String("image".to_string()))
            .and_then(Value::as_str)
        {
            entries.push(ServiceEntry {
                path: path.to_path_buf(),
                service: service_name.to_string(),
                image: image.to_string(),
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

        let web_service = results.iter().find(|s| s.service == "web").unwrap();
        assert_eq!(web_service.image, "nginx:latest");

        let db_service = results.iter().find(|s| s.service == "db").unwrap();
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

        let api_service = result.iter().find(|s| s.service == "api").unwrap();
        assert_eq!(api_service.image, "myapi:1.0");

        let cache_service = result.iter().find(|s| s.service == "cache").unwrap();
        assert_eq!(cache_service.image, "redis:latest");

        dir.close().unwrap();
    }
}
