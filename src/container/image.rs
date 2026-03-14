use color_eyre::Result;
use eyre::eyre;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImageReference {
    // Full image reference, e.g. "docker.io/library/ubuntu:latest", "ghcr.io/myorg/myimage:1.0.0", "quay.io/coreos/tectonic-console:v2.9.0-tectonic.1"
    pub repository: String,

    // Image tag, e.g. "latest", "1.0.0", "v2.9.0-tectonic.1"
    pub tag: ImageTag,

    // Optional digest for content-addressable image references, e.g. "sha256:abc123..."
    pub digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImageTag(String);

impl ImageTag {
    pub fn new(tag: impl Into<String>) -> Self {
        Self(tag.into())
    }

    pub fn is_latest(&self) -> bool {
        self.0 == "latest"
    }

    pub fn is_semver(&self) -> bool {
        // A simple heuristic to check if the tag looks like a version, e.g. "v1.2.3", "1.0.0"
        (self.0.starts_with('v')
            && self.0[1..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit()))
            || self.0.chars().next().is_some_and(|c| c.is_ascii_digit())
    }
}

impl fmt::Display for ImageTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl ImageReference {
    // Parses an image reference string into an ImageReference struct.
    pub fn parse(image: &str) -> Result<Self> {
        let (image_part, digest) = if let Some(pos) = image.find('@') {
            let (img, digest) = image.split_at(pos);
            (img, Some(digest[1..].to_string()))
        } else {
            (image, None)
        };

        let (repository, tag) = if let Some(pos) = image_part.rfind(':') {
            let after_colon = &image_part[pos + 1..];
            if after_colon.contains('/') {
                // this is a port, not a tag
                (image_part.to_string(), ImageTag::new("latest"))
            } else {
                let (repo, tag) = image_part.split_at(pos);
                (repo.to_string(), ImageTag::new(&tag[1..]))
            }
        } else {
            (image_part.to_string(), ImageTag::new("latest"))
        };

        if repository.is_empty() {
            return Err(eyre!("Invalid image reference: repository is empty"));
        }

        Ok(Self {
            repository,
            tag,
            digest,
        })
    }
}

impl fmt::Display for ImageReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.repository, self.tag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple() {
        let img = ImageReference::parse("nginx").unwrap();
        assert_eq!(img.repository, "nginx");
        assert_eq!(img.tag.to_string(), "latest");
        assert_eq!(img.digest, None);
    }

    #[test]
    fn test_parse_with_tag() {
        let img = ImageReference::parse("nginx:1.25").unwrap();
        assert_eq!(img.repository, "nginx");
        assert_eq!(img.tag.to_string(), "1.25");
    }

    #[test]
    fn test_parse_with_registry() {
        let img = ImageReference::parse("docker.io/library/nginx:alpine").unwrap();
        assert_eq!(img.repository, "docker.io/library/nginx");
        assert_eq!(img.tag.to_string(), "alpine");
    }

    #[test]
    fn test_parse_with_digest() {
        let img = ImageReference::parse("nginx@sha256:abcd1234").unwrap();
        assert_eq!(img.repository, "nginx");
        assert_eq!(img.digest, Some("sha256:abcd1234".to_string()));
    }

    #[test]
    fn test_parse_registry_with_port() {
        let img = ImageReference::parse("localhost:5000/myapp:v1").unwrap();
        assert_eq!(img.repository, "localhost:5000/myapp");
        assert_eq!(img.tag.to_string(), "v1");
    }

    #[test]
    fn test_to_string() {
        let img = ImageReference {
            repository: "nginx".to_string(),
            tag: ImageTag::new("1.25"),
            digest: None,
        };
        assert_eq!(img.to_string(), "nginx:1.25");
    }

    #[test]
    fn test_tag_is_semver() {
        assert!(ImageTag::new("v1.2.3").is_semver());
        assert!(ImageTag::new("1.2.3").is_semver());
        assert!(!ImageTag::new("latest").is_semver());
        assert!(!ImageTag::new("alpine").is_semver());
    }
}
