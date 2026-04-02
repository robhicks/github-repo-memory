use anyhow::Result;
use crate::graph::models::Dependency;

/// Parse dependencies from a manifest file based on its filename.
pub fn parse_manifest(filename: &str, content: &str) -> Result<Vec<Dependency>> {
    let lower = filename.to_lowercase();
    if lower.ends_with("cargo.toml") {
        parse_cargo_toml(content)
    } else if lower.ends_with("package.json") {
        parse_package_json(content)
    } else if lower.ends_with("go.mod") {
        parse_go_mod(content)
    } else if lower.ends_with("requirements.txt") {
        parse_requirements_txt(content)
    } else if lower.ends_with("pyproject.toml") {
        parse_pyproject_toml(content)
    } else {
        Ok(vec![])
    }
}

fn parse_cargo_toml(content: &str) -> Result<Vec<Dependency>> {
    let parsed: toml::Value = toml::from_str(content)?;
    let mut deps = Vec::new();

    for (section, is_dev) in [("dependencies", false), ("dev-dependencies", true)] {
        if let Some(table) = parsed.get(section).and_then(|v| v.as_table()) {
            for (name, value) in table {
                let version = match value {
                    toml::Value::String(v) => Some(v.clone()),
                    toml::Value::Table(t) => {
                        t.get("version").and_then(|v| v.as_str()).map(String::from)
                    }
                    _ => None,
                };
                deps.push(Dependency {
                    name: name.clone(),
                    version_spec: version,
                    ecosystem: "crates".to_string(),
                });
                let _ = is_dev; // TODO: track dev deps separately if needed
            }
        }
    }

    Ok(deps)
}

fn parse_package_json(content: &str) -> Result<Vec<Dependency>> {
    let parsed: serde_json::Value = serde_json::from_str(content)?;
    let mut deps = Vec::new();

    for section in ["dependencies", "devDependencies"] {
        if let Some(obj) = parsed.get(section).and_then(|v| v.as_object()) {
            for (name, version) in obj {
                deps.push(Dependency {
                    name: name.clone(),
                    version_spec: version.as_str().map(String::from),
                    ecosystem: "npm".to_string(),
                });
            }
        }
    }

    Ok(deps)
}

fn parse_go_mod(content: &str) -> Result<Vec<Dependency>> {
    let mut deps = Vec::new();
    let mut in_require = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("require (") || trimmed == "require (" {
            in_require = true;
            continue;
        }
        if trimmed == ")" {
            in_require = false;
            continue;
        }
        if in_require {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 2 {
                deps.push(Dependency {
                    name: parts[0].to_string(),
                    version_spec: Some(parts[1].to_string()),
                    ecosystem: "go".to_string(),
                });
            }
        } else if trimmed.starts_with("require ") {
            let rest = trimmed.strip_prefix("require ").unwrap_or("");
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 2 {
                deps.push(Dependency {
                    name: parts[0].to_string(),
                    version_spec: Some(parts[1].to_string()),
                    ecosystem: "go".to_string(),
                });
            }
        }
    }

    Ok(deps)
}

fn parse_requirements_txt(content: &str) -> Result<Vec<Dependency>> {
    let mut deps = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('-') {
            continue;
        }

        // Handle various specifiers: ==, >=, <=, ~=, !=, >
        let (name, version) = if let Some(pos) = trimmed.find(|c: char| c == '=' || c == '>' || c == '<' || c == '~' || c == '!') {
            (&trimmed[..pos], Some(trimmed[pos..].to_string()))
        } else {
            (trimmed, None)
        };

        if !name.is_empty() {
            deps.push(Dependency {
                name: name.trim().to_string(),
                version_spec: version,
                ecosystem: "pypi".to_string(),
            });
        }
    }

    Ok(deps)
}

fn parse_pyproject_toml(content: &str) -> Result<Vec<Dependency>> {
    let parsed: toml::Value = toml::from_str(content)?;
    let mut deps = Vec::new();

    // PEP 621: [project].dependencies
    if let Some(project_deps) = parsed
        .get("project")
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_array())
    {
        for dep in project_deps {
            if let Some(dep_str) = dep.as_str() {
                let (name, version) = parse_pep508(dep_str);
                deps.push(Dependency {
                    name,
                    version_spec: version,
                    ecosystem: "pypi".to_string(),
                });
            }
        }
    }

    Ok(deps)
}

/// Parse a PEP 508 dependency specifier into (name, version_spec).
fn parse_pep508(spec: &str) -> (String, Option<String>) {
    let trimmed = spec.trim();
    if let Some(pos) = trimmed.find(|c: char| c == '>' || c == '<' || c == '=' || c == '~' || c == '!' || c == '[') {
        let name = trimmed[..pos].trim().to_string();
        let rest = trimmed[pos..].to_string();
        (name, Some(rest))
    } else {
        (trimmed.to_string(), None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cargo_toml() {
        let content = r#"
[package]
name = "my-app"
version = "0.1.0"

[dependencies]
serde = "1.0"
tokio = { version = "1", features = ["full"] }
anyhow = "1"

[dev-dependencies]
criterion = "0.5"
"#;
        let deps = parse_cargo_toml(content).unwrap();
        assert_eq!(deps.len(), 4);
        assert!(deps.iter().any(|d| d.name == "serde" && d.version_spec.as_deref() == Some("1.0")));
        assert!(deps.iter().any(|d| d.name == "tokio" && d.version_spec.as_deref() == Some("1")));
        assert!(deps.iter().any(|d| d.name == "criterion"));
        assert!(deps.iter().all(|d| d.ecosystem == "crates"));
    }

    #[test]
    fn test_parse_package_json() {
        let content = r#"{
  "name": "my-app",
  "dependencies": {
    "express": "^4.18.0",
    "lodash": "^4.17.21"
  },
  "devDependencies": {
    "jest": "^29.0.0"
  }
}"#;
        let deps = parse_package_json(content).unwrap();
        assert_eq!(deps.len(), 3);
        assert!(deps.iter().any(|d| d.name == "express" && d.version_spec.as_deref() == Some("^4.18.0")));
        assert!(deps.iter().any(|d| d.name == "jest"));
        assert!(deps.iter().all(|d| d.ecosystem == "npm"));
    }

    #[test]
    fn test_parse_go_mod() {
        let content = r#"module github.com/myorg/myapp

go 1.21

require (
	github.com/gin-gonic/gin v1.9.1
	github.com/stretchr/testify v1.8.4
)

require github.com/single/dep v0.1.0
"#;
        let deps = parse_go_mod(content).unwrap();
        assert_eq!(deps.len(), 3);
        assert!(deps.iter().any(|d| d.name == "github.com/gin-gonic/gin" && d.version_spec.as_deref() == Some("v1.9.1")));
        assert!(deps.iter().any(|d| d.name == "github.com/single/dep"));
        assert!(deps.iter().all(|d| d.ecosystem == "go"));
    }

    #[test]
    fn test_parse_requirements_txt() {
        let content = r#"
# This is a comment
flask==2.3.0
requests>=2.28.0
numpy
-e git+https://github.com/something
pandas~=1.5
"#;
        let deps = parse_requirements_txt(content).unwrap();
        assert_eq!(deps.len(), 4);
        assert!(deps.iter().any(|d| d.name == "flask" && d.version_spec.as_deref() == Some("==2.3.0")));
        assert!(deps.iter().any(|d| d.name == "requests" && d.version_spec.as_deref() == Some(">=2.28.0")));
        assert!(deps.iter().any(|d| d.name == "numpy" && d.version_spec.is_none()));
        assert!(deps.iter().any(|d| d.name == "pandas"));
        assert!(deps.iter().all(|d| d.ecosystem == "pypi"));
    }

    #[test]
    fn test_parse_pyproject_toml() {
        let content = r#"
[project]
name = "my-project"
dependencies = [
    "fastapi>=0.100.0",
    "uvicorn[standard]",
    "pydantic",
]
"#;
        let deps = parse_pyproject_toml(content).unwrap();
        assert_eq!(deps.len(), 3);
        assert!(deps.iter().any(|d| d.name == "fastapi" && d.version_spec.is_some()));
        assert!(deps.iter().any(|d| d.name == "uvicorn" && d.version_spec.as_deref() == Some("[standard]")));
        assert!(deps.iter().any(|d| d.name == "pydantic" && d.version_spec.is_none()));
    }

    #[test]
    fn test_parse_manifest_dispatch() {
        let cargo = parse_manifest("Cargo.toml", "[dependencies]\nserde = \"1\"").unwrap();
        assert_eq!(cargo.len(), 1);
        assert_eq!(cargo[0].ecosystem, "crates");

        let npm = parse_manifest("package.json", r#"{"dependencies":{"a":"1"}}"#).unwrap();
        assert_eq!(npm.len(), 1);
        assert_eq!(npm[0].ecosystem, "npm");

        let unknown = parse_manifest("Makefile", "something").unwrap();
        assert!(unknown.is_empty());
    }

    #[test]
    fn test_parse_pep508() {
        assert_eq!(parse_pep508("flask>=2.0"), ("flask".to_string(), Some(">=2.0".to_string())));
        assert_eq!(parse_pep508("numpy"), ("numpy".to_string(), None));
        assert_eq!(parse_pep508("uvicorn[standard]"), ("uvicorn".to_string(), Some("[standard]".to_string())));
    }
}
