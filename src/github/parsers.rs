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
