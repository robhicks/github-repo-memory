use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Organization {
    pub login: String,
    pub name: Option<String>,
    pub url: String,
    pub last_sync_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    pub full_name: String,
    pub name: String,
    pub description: Option<String>,
    pub default_branch: String,
    pub is_archived: bool,
    pub is_fork: bool,
    pub stars: u32,
    pub updated_at: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Language {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    pub name: String,
    pub version_spec: Option<String>,
    pub ecosystem: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Topic {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Team {
    pub slug: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileNode {
    pub path: String,
    pub repo: String,
    pub kind: FileKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FileKind {
    Config,
    Readme,
    Source,
    Manifest,
    Ci,
    Other,
}

impl std::fmt::Display for FileKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileKind::Config => write!(f, "config"),
            FileKind::Readme => write!(f, "readme"),
            FileKind::Source => write!(f, "source"),
            FileKind::Manifest => write!(f, "manifest"),
            FileKind::Ci => write!(f, "ci"),
            FileKind::Other => write!(f, "other"),
        }
    }
}

impl FileKind {
    pub fn from_path(path: &str) -> Self {
        let lower = path.to_lowercase();
        if lower.contains("readme") {
            FileKind::Readme
        } else if lower.ends_with("cargo.toml")
            || lower.ends_with("package.json")
            || lower.ends_with("go.mod")
            || lower.ends_with("requirements.txt")
            || lower.ends_with("pyproject.toml")
            || lower.ends_with("pom.xml")
            || lower.ends_with("build.gradle")
            || lower.ends_with("gemfile")
        {
            FileKind::Manifest
        } else if lower.contains(".github/workflows")
            || lower.contains(".gitlab-ci")
            || lower.ends_with("jenkinsfile")
        {
            FileKind::Ci
        } else if lower.ends_with(".toml")
            || lower.ends_with(".yaml")
            || lower.ends_with(".yml")
            || lower.ends_with(".json")
            || lower.ends_with(".ini")
        {
            FileKind::Config
        } else {
            FileKind::Other
        }
    }
}
