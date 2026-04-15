use std::{fs, time::SystemTime};

use anyhow::{Context, bail};
use camino::{Utf8Path, Utf8PathBuf};
use regex::Regex;

use crate::{
    RuntimeConfig,
    infra::{
        buckets::{bucket_root, known_bucket_repos, local_bucket_names},
        git::{clone_single_branch, git_available, latest_commit_time, ls_remote, origin_url},
        sqlite_cache::rebuild_search_cache,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BucketListEntry {
    pub name: String,
    pub source: String,
    pub updated: SystemTime,
    pub manifests: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BucketAddOutcome {
    Added {
        name: String,
    },
    AlreadyExists {
        name: String,
    },
    DuplicateRemote {
        name: String,
        existing_bucket: String,
    },
}

pub fn known_bucket_names(config: &RuntimeConfig) -> anyhow::Result<Vec<String>> {
    Ok(known_bucket_repos(config)?
        .into_iter()
        .map(|(name, _)| name)
        .collect())
}

pub fn list_buckets(config: &RuntimeConfig) -> anyhow::Result<Vec<BucketListEntry>> {
    let mut rows = Vec::new();
    for name in local_bucket_names(config)? {
        let root = bucket_root(config, Some(&name));
        let manifests_root = root.join("bucket");
        let is_git_repo = root.join(".git").exists() && git_available();
        let source = if is_git_repo {
            origin_url(&root).unwrap_or_else(|_| root.as_str().to_owned())
        } else {
            root.as_str().to_owned()
        };
        let updated = if is_git_repo {
            latest_commit_time(&root)?
        } else {
            metadata_modified_time(if manifests_root.is_dir() {
                manifests_root.as_path()
            } else {
                root.as_path()
            })?
        };
        rows.push(BucketListEntry {
            manifests: count_manifests(&manifests_root)?,
            name,
            source,
            updated,
        });
    }
    Ok(rows)
}

pub fn add_bucket(
    config: &RuntimeConfig,
    name: &str,
    repo: Option<&str>,
) -> anyhow::Result<BucketAddOutcome> {
    if !git_available() {
        bail!("Git is required for buckets. Run 'scoop install git' and try again.");
    }

    let destination = bucket_root(config, Some(name));
    if destination.exists() {
        return Ok(BucketAddOutcome::AlreadyExists {
            name: name.to_owned(),
        });
    }

    let remote = match repo {
        Some(repo) => repo.to_owned(),
        None => known_bucket_repo(config, name)?
            .ok_or_else(|| anyhow::anyhow!("Unknown bucket '{name}'. Try specifying <repo>."))?,
    };
    let normalized_remote = normalize_repository_uri(&remote)?;

    for existing_bucket in local_bucket_names(config)? {
        let existing_root = bucket_root(config, Some(&existing_bucket));
        if !existing_root.join(".git").exists() {
            continue;
        }
        let existing_remote = match origin_url(&existing_root) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if normalize_repository_uri(&existing_remote).ok().as_deref() == Some(&normalized_remote) {
            return Ok(BucketAddOutcome::DuplicateRemote {
                name: name.to_owned(),
                existing_bucket,
            });
        }
    }

    ls_remote(&remote)
        .with_context(|| format!("'{remote}' doesn't look like a valid git repository"))?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create buckets directory {}", parent))?;
    }
    if let Err(error) = clone_single_branch(&remote, None, &destination) {
        let _ = fs::remove_dir_all(&destination);
        return Err(error.context(format!("Failed to clone '{remote}' to '{destination}'.")));
    }

    if config.settings().use_sqlite_cache.unwrap_or(false) {
        rebuild_search_cache(config)?;
    }

    Ok(BucketAddOutcome::Added {
        name: name.to_owned(),
    })
}

pub fn remove_bucket(config: &RuntimeConfig, name: &str) -> anyhow::Result<bool> {
    let root = bucket_root(config, Some(name));
    if !root.exists() {
        return Ok(false);
    }
    fs::remove_dir_all(&root).with_context(|| format!("failed to remove bucket {}", root))?;
    if config.settings().use_sqlite_cache.unwrap_or(false) {
        rebuild_search_cache(config)?;
    }
    Ok(true)
}

fn known_bucket_repo(config: &RuntimeConfig, name: &str) -> anyhow::Result<Option<String>> {
    Ok(known_bucket_repos(config)?
        .into_iter()
        .find(|(bucket_name, _)| bucket_name == name)
        .map(|(_, repo)| repo))
}

fn metadata_modified_time(path: &Utf8Path) -> anyhow::Result<SystemTime> {
    fs::metadata(path)
        .with_context(|| format!("failed to read metadata from {}", path))?
        .modified()
        .with_context(|| format!("failed to read modified time from {}", path))
}

fn count_manifests(root: &Utf8Path) -> anyhow::Result<usize> {
    if !root.is_dir() {
        return Ok(0);
    }
    let mut count = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        for entry in
            fs::read_dir(&path).with_context(|| format!("failed to read bucket dir {}", path))?
        {
            let entry = entry?;
            let entry_path = Utf8PathBuf::from_path_buf(entry.path())
                .map_err(|_| anyhow::anyhow!("bucket path must be valid UTF-8"))?;
            let file_type = entry.file_type().with_context(|| {
                format!("failed to read file type for bucket entry {}", entry_path)
            })?;
            if file_type.is_dir() {
                stack.push(entry_path);
            } else if file_type.is_file()
                && entry_path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
            {
                count += 1;
            }
        }
    }
    Ok(count)
}

fn normalize_repository_uri(uri: &str) -> anyhow::Result<String> {
    if let Ok(path) = fs::canonicalize(uri) {
        return Ok(path.to_string_lossy().to_ascii_lowercase());
    }
    let pattern = Regex::new(
        r"(?:@|/{1,3})(?:www\.|.*@)?(?P<provider>[^/]+?)(?::\d+)?[:/](?P<user>.+)/(?P<repo>.+?)(?:\.git)?/?$",
    )
    .expect("bucket repository regex should compile");
    let captures = pattern
        .captures(uri)
        .ok_or_else(|| anyhow::anyhow!("{uri} is not a valid Git URL"))?;
    Ok(format!(
        "{}/{}/{}",
        captures["provider"].to_ascii_lowercase(),
        &captures["user"],
        &captures["repo"]
    ))
}

#[cfg(test)]
mod tests {
    use std::{fs, process::Command};

    use camino::{Utf8Path, Utf8PathBuf};
    use tempfile::TempDir;

    use crate::RuntimeConfig;

    use super::{BucketAddOutcome, add_bucket, known_bucket_names, normalize_repository_uri};

    #[test]
    fn normalizes_http_and_ssh_urls_to_same_key() {
        let http = normalize_repository_uri("https://github.com/ScoopInstaller/Extras.git")
            .expect("http repo should normalize");
        let ssh = normalize_repository_uri("git@github.com:ScoopInstaller/Extras.git")
            .expect("ssh repo should normalize");
        assert_eq!(http, ssh);
    }

    #[test]
    fn reads_known_bucket_names_from_embedded_file() {
        let fixture = BucketFixture::new();
        fixture.seed_scoop_buckets_json(r#"{"main":"https://github.com/ScoopInstaller/Main"}"#);

        let names = known_bucket_names(&fixture.config()).expect("known buckets should load");
        assert_eq!(names, vec![String::from("main")]);
    }

    #[test]
    fn clones_bucket_repository_into_local_bucket_dir() {
        let fixture = BucketFixture::new();
        let remote = fixture
            .create_remote_git_repo("extras", &[("bucket/demo.json", r#"{"version":"1.0.0"}"#)]);

        let outcome =
            add_bucket(&fixture.config(), "extras", Some(&remote)).expect("bucket add succeeds");

        assert_eq!(
            outcome,
            BucketAddOutcome::Added {
                name: String::from("extras")
            }
        );
        assert!(
            fixture
                .config()
                .paths()
                .buckets()
                .join("extras")
                .join("bucket")
                .join("demo.json")
                .is_file()
        );
    }

    struct BucketFixture {
        _temp: TempDir,
        root: Utf8PathBuf,
    }

    impl BucketFixture {
        fn new() -> Self {
            let temp = TempDir::new().expect("temp dir should be created");
            let root = Utf8PathBuf::from_path_buf(temp.path().to_path_buf())
                .expect("temp path should be valid UTF-8");
            fs::create_dir_all(root.join("local")).expect("local root should exist");
            fs::create_dir_all(root.join("global")).expect("global root should exist");
            Self { _temp: temp, root }
        }

        fn config(&self) -> RuntimeConfig {
            RuntimeConfig::new(self.root.join("local"), self.root.join("global"))
        }

        fn seed_scoop_buckets_json(&self, content: &str) {
            let path = self
                .config()
                .paths()
                .current_dir("scoop")
                .join("buckets.json");
            fs::create_dir_all(path.parent().expect("buckets.json should have a parent"))
                .expect("buckets.json parent should exist");
            fs::write(path, content).expect("buckets json should exist");
        }

        fn create_remote_git_repo(&self, name: &str, files: &[(&str, &str)]) -> String {
            let source = self.root.join(format!("{name}-src"));
            let remote = self.root.join(format!("{name}-remote.git"));
            fs::create_dir_all(&source).expect("source repo should exist");
            run_git(&source, &["init"]);
            run_git(&source, &["config", "user.name", "Codex"]);
            run_git(&source, &["config", "user.email", "codex@example.invalid"]);
            run_git(&source, &["config", "commit.gpgsign", "false"]);
            for (relative, content) in files {
                let path = source.join(relative);
                fs::create_dir_all(path.parent().expect("file should have a parent"))
                    .expect("file parent should exist");
                fs::write(path, content).expect("fixture file should exist");
            }
            run_git(&source, &["add", "."]);
            run_git(&source, &["commit", "-m", "seed"]);
            run_git(self.root.as_path(), &["init", "--bare", remote.as_str()]);
            run_git(&source, &["remote", "add", "origin", remote.as_str()]);
            let branch = git_stdout(&source, &["branch", "--show-current"]);
            run_git(&source, &["push", "-u", "origin", branch.trim()]);
            remote.to_string()
        }
    }

    fn run_git(cwd: &Utf8Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(cwd)
            .args(args)
            .output()
            .unwrap_or_else(|error| panic!("failed to run git {:?}: {error}", args));
        assert!(
            output.status.success(),
            "git {:?} failed\nstdout: {}\nstderr: {}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_stdout(cwd: &Utf8Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(cwd)
            .args(args)
            .output()
            .unwrap_or_else(|error| panic!("failed to run git {:?}: {error}", args));
        assert!(
            output.status.success(),
            "git {:?} failed\nstdout: {}\nstderr: {}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }
}
