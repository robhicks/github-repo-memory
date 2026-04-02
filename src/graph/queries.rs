/// Cypher query constants for graph operations.
/// All write operations use MERGE for idempotency.

// --- Schema ---

pub const CREATE_REPO_INDEX: &str = "CREATE INDEX FOR (r:Repository) ON (r.full_name)";
pub const CREATE_REPO_NAME_INDEX: &str = "CREATE INDEX FOR (r:Repository) ON (r.name)";
pub const CREATE_LANG_INDEX: &str = "CREATE INDEX FOR (l:Language) ON (l.name)";
pub const CREATE_DEP_INDEX: &str = "CREATE INDEX FOR (d:Dependency) ON (d.name)";
pub const CREATE_TOPIC_INDEX: &str = "CREATE INDEX FOR (t:Topic) ON (t.name)";
pub const CREATE_TEAM_INDEX: &str = "CREATE INDEX FOR (tm:Team) ON (tm.slug)";
pub const CREATE_ORG_INDEX: &str = "CREATE INDEX FOR (o:Organization) ON (o.login)";

pub const SCHEMA_INDICES: &[&str] = &[
    CREATE_REPO_INDEX,
    CREATE_REPO_NAME_INDEX,
    CREATE_LANG_INDEX,
    CREATE_DEP_INDEX,
    CREATE_TOPIC_INDEX,
    CREATE_TEAM_INDEX,
    CREATE_ORG_INDEX,
];

// --- Merge operations ---

pub const MERGE_ORG: &str = r#"
MERGE (o:Organization {login: $login})
SET o.name = $name, o.url = $url
RETURN o
"#;

pub const UPDATE_ORG_SYNC_TIME: &str = r#"
MATCH (o:Organization {login: $login})
SET o.last_sync_at = $last_sync_at
RETURN o
"#;

pub const MERGE_REPO: &str = r#"
MERGE (r:Repository {full_name: $full_name})
SET r.name = $name,
    r.description = $description,
    r.default_branch = $default_branch,
    r.is_archived = $is_archived,
    r.is_fork = $is_fork,
    r.stars = $stars,
    r.updated_at = $updated_at,
    r.url = $url
RETURN r
"#;

pub const MERGE_ORG_HAS_REPO: &str = r#"
MATCH (o:Organization {login: $org_login})
MATCH (r:Repository {full_name: $repo_full_name})
MERGE (o)-[:HAS_REPO]->(r)
"#;

pub const MERGE_LANGUAGE: &str = r#"
MERGE (l:Language {name: $name})
RETURN l
"#;

pub const MERGE_REPO_USES_LANGUAGE: &str = r#"
MATCH (r:Repository {full_name: $repo_full_name})
MERGE (l:Language {name: $lang_name})
MERGE (r)-[rel:USES_LANGUAGE]->(l)
SET rel.bytes = $bytes
"#;

pub const MERGE_DEPENDENCY: &str = r#"
MATCH (r:Repository {full_name: $repo_full_name})
MERGE (d:Dependency {name: $dep_name, ecosystem: $ecosystem})
MERGE (r)-[rel:HAS_DEPENDENCY]->(d)
SET rel.version = $version, rel.dev = $dev
"#;

pub const MERGE_TOPIC: &str = r#"
MATCH (r:Repository {full_name: $repo_full_name})
MERGE (t:Topic {name: $topic_name})
MERGE (r)-[:HAS_TOPIC]->(t)
"#;

pub const MERGE_TEAM: &str = r#"
MATCH (r:Repository {full_name: $repo_full_name})
MERGE (tm:Team {slug: $team_slug})
SET tm.name = $team_name
MERGE (r)-[rel:OWNED_BY]->(tm)
SET rel.permission = $permission
"#;

pub const MERGE_FILE: &str = r#"
MATCH (r:Repository {full_name: $repo_full_name})
MERGE (f:File {path: $path, repo: $repo_full_name})
SET f.kind = $kind
MERGE (r)-[:HAS_FILE]->(f)
"#;

pub const MERGE_CROSS_REPO_DEPENDENCY: &str = r#"
MATCH (source:Repository)-[:HAS_DEPENDENCY]->(d:Dependency)
MATCH (target:Repository)
WHERE d.name = target.name AND source.full_name <> target.full_name
MERGE (source)-[rel:DEPENDS_ON_REPO]->(target)
SET rel.via = d.name
"#;

// --- Query operations ---

pub const SEARCH_REPOS: &str = r#"
MATCH (r:Repository)
WHERE r.name CONTAINS $query
RETURN r.full_name, r.name, r.description, r.stars, r.updated_at, r.url, r.is_archived
ORDER BY r.stars DESC
LIMIT $limit
"#;

pub const SEARCH_REPOS_BY_LANGUAGE: &str = r#"
MATCH (r:Repository)-[:USES_LANGUAGE]->(l:Language {name: $language})
RETURN r.full_name, r.name, r.description, r.stars, r.updated_at, r.url
ORDER BY r.stars DESC
LIMIT $limit
"#;

pub const SEARCH_REPOS_BY_TOPIC: &str = r#"
MATCH (r:Repository)-[:HAS_TOPIC]->(t:Topic {name: $topic})
RETURN r.full_name, r.name, r.description, r.stars, r.updated_at, r.url
ORDER BY r.stars DESC
LIMIT $limit
"#;

pub const SEARCH_REPOS_BY_DEPENDENCY: &str = r#"
MATCH (r:Repository)-[:HAS_DEPENDENCY]->(d:Dependency {name: $dependency})
RETURN r.full_name, r.name, r.description, r.stars, r.updated_at, d.ecosystem
ORDER BY r.stars DESC
LIMIT $limit
"#;

pub const GET_REPO_DETAILS: &str = r#"
MATCH (r:Repository {full_name: $full_name})
OPTIONAL MATCH (r)-[ul:USES_LANGUAGE]->(l:Language)
OPTIONAL MATCH (r)-[:HAS_TOPIC]->(t:Topic)
OPTIONAL MATCH (r)-[:OWNED_BY]->(tm:Team)
OPTIONAL MATCH (r)-[:HAS_DEPENDENCY]->(d:Dependency)
OPTIONAL MATCH (dependent:Repository)-[:DEPENDS_ON_REPO]->(r)
RETURN r,
       collect(DISTINCT l.name) AS languages,
       collect(DISTINCT t.name) AS topics,
       collect(DISTINCT tm.name) AS teams,
       count(DISTINCT d) AS dependency_count,
       count(DISTINCT dependent) AS dependent_count
"#;

pub const FIND_DEPENDENTS: &str = r#"
MATCH (r:Repository)-[:HAS_DEPENDENCY]->(d:Dependency {name: $dependency})
RETURN r.full_name, r.description, d.ecosystem
ORDER BY r.full_name
LIMIT $limit
"#;

pub const FIND_RELATED_BY_DEPS: &str = r#"
MATCH (source:Repository {full_name: $repo})-[:HAS_DEPENDENCY]->(d:Dependency)<-[:HAS_DEPENDENCY]-(other:Repository)
WHERE other.full_name <> $repo
RETURN other.full_name, other.description, count(d) AS shared_deps
ORDER BY shared_deps DESC
LIMIT $limit
"#;

pub const FIND_RELATED_BY_TEAM: &str = r#"
MATCH (source:Repository {full_name: $repo})-[:OWNED_BY]->(tm:Team)<-[:OWNED_BY]-(other:Repository)
WHERE other.full_name <> $repo
RETURN other.full_name, other.description, tm.name AS team
ORDER BY other.full_name
LIMIT $limit
"#;

pub const FIND_RELATED_BY_TOPIC: &str = r#"
MATCH (source:Repository {full_name: $repo})-[:HAS_TOPIC]->(t:Topic)<-[:HAS_TOPIC]-(other:Repository)
WHERE other.full_name <> $repo
RETURN other.full_name, other.description, count(t) AS shared_topics
ORDER BY shared_topics DESC
LIMIT $limit
"#;

pub const EXPLORE_DEPS_UPSTREAM: &str = r#"
MATCH path = (r:Repository {full_name: $repo})-[:DEPENDS_ON_REPO*1..3]->(dep:Repository)
RETURN [n IN nodes(path) | n.full_name] AS chain
LIMIT $limit
"#;

pub const EXPLORE_DEPS_DOWNSTREAM: &str = r#"
MATCH path = (dep:Repository)-[:DEPENDS_ON_REPO*1..3]->(r:Repository {full_name: $repo})
RETURN [n IN nodes(path) | n.full_name] AS chain
LIMIT $limit
"#;

pub const LIST_LANGUAGES: &str = r#"
MATCH (r:Repository)-[:USES_LANGUAGE]->(l:Language)
RETURN l.name, count(r) AS repo_count, sum(r.stars) AS total_stars
ORDER BY repo_count DESC
"#;

pub const LIST_TEAMS: &str = r#"
MATCH (r:Repository)-[:OWNED_BY]->(tm:Team)
RETURN tm.name, tm.slug, count(r) AS repo_count
ORDER BY repo_count DESC
"#;

pub const GET_ORG_STATS: &str = r#"
MATCH (o:Organization {login: $org})
OPTIONAL MATCH (o)-[:HAS_REPO]->(r:Repository)
WITH o, count(r) AS total_repos
OPTIONAL MATCH (:Repository)-[:USES_LANGUAGE]->(l:Language)
WITH o, total_repos, count(DISTINCT l) AS total_languages
OPTIONAL MATCH (:Repository)-[:HAS_DEPENDENCY]->(d:Dependency)
WITH o, total_repos, total_languages, count(DISTINCT d) AS total_dependencies
OPTIONAL MATCH (:Repository)-[:OWNED_BY]->(tm:Team)
RETURN o.login, o.last_sync_at, total_repos, total_languages, total_dependencies, count(DISTINCT tm) AS total_teams
"#;
