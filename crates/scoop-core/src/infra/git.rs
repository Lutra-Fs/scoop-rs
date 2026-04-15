use std::{process::Command, time::SystemTime};

use anyhow::{Context, bail};
use camino::Utf8Path;
use jiff::Timestamp;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UpdateStatus {
    pub needs_update: bool,
    pub network_failure: bool,
}

pub trait GitBackend: Send + Sync {
    fn is_available(&self) -> bool;
    fn test_update_status(&self, repopath: &Utf8Path) -> UpdateStatus;
    fn latest_author_for_path(&self, path: &Utf8Path) -> Option<String>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ExternalGitBackend;

static EXTERNAL_GIT_BACKEND: ExternalGitBackend = ExternalGitBackend;

pub fn system_backend() -> &'static ExternalGitBackend {
    &EXTERNAL_GIT_BACKEND
}

pub fn git_available() -> bool {
    system_backend().is_available()
}

pub fn test_update_status(repopath: &Utf8Path) -> UpdateStatus {
    system_backend().test_update_status(repopath)
}

pub fn latest_author_for_path(path: &Utf8Path) -> Option<String> {
    system_backend().latest_author_for_path(path)
}

pub fn current_branch(repopath: &Utf8Path) -> anyhow::Result<String> {
    let branch = git_stdout(repopath, &["branch", "--show-current"])?;
    let branch = branch.trim();
    if branch.is_empty() {
        bail!("failed to determine current git branch for {}", repopath);
    }
    Ok(branch.to_owned())
}

pub fn origin_url(repopath: &Utf8Path) -> anyhow::Result<String> {
    let remote = git_stdout(repopath, &["config", "--get", "remote.origin.url"])?;
    let remote = remote.trim();
    if remote.is_empty() {
        bail!("failed to determine git remote.origin.url for {}", repopath);
    }
    Ok(remote.to_owned())
}

pub fn set_origin_url(repopath: &Utf8Path, remote_url: &str) -> anyhow::Result<()> {
    run_git(repopath, &["config", "remote.origin.url", remote_url])
}

pub fn clone_single_branch(
    remote_url: &str,
    branch: Option<&str>,
    destination: &Utf8Path,
) -> anyhow::Result<()> {
    let parent = destination
        .parent()
        .with_context(|| format!("{} has no parent directory", destination))?;
    let mut args = vec!["clone", "-q", remote_url];
    if let Some(branch) = branch.filter(|branch| !branch.is_empty()) {
        args.extend(["--branch", branch, "--single-branch"]);
    }
    args.push(destination.as_str());
    run_git(parent, &args)
}

pub fn force_sync_branch(
    repopath: &Utf8Path,
    remote_url: Option<&str>,
    branch: Option<&str>,
) -> anyhow::Result<()> {
    if let Some(remote_url) = remote_url.filter(|remote| !remote.is_empty()) {
        let current_remote = origin_url(repopath).ok();
        if current_remote.as_deref() != Some(remote_url) {
            set_origin_url(repopath, remote_url)?;
        }
    }

    let branch = match branch.filter(|branch| !branch.is_empty()) {
        Some(branch) => branch.to_owned(),
        None => current_branch(repopath)?,
    };

    run_git(
        repopath,
        &[
            "fetch",
            "--force",
            "origin",
            &format!("refs/heads/{branch}:refs/remotes/origin/{branch}"),
            "-q",
        ],
    )?;
    run_git(repopath, &["checkout", "-B", &branch, "-q"])?;
    run_git(
        repopath,
        &["reset", "--hard", &format!("origin/{branch}"), "-q"],
    )
}

pub fn pull_tags_force(repopath: &Utf8Path) -> anyhow::Result<()> {
    run_git(repopath, &["pull", "--tags", "--force", "-q"])
}

pub fn ls_remote(remote_url: &str) -> anyhow::Result<()> {
    let output = Command::new("git")
        .args(["ls-remote", remote_url])
        .output()
        .with_context(|| format!("failed to run git ls-remote for {remote_url}"))?;
    if !output.status.success() {
        bail!(
            "git ls-remote failed for {}\nstdout: {}\nstderr: {}",
            remote_url,
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

pub fn latest_commit_time(repopath: &Utf8Path) -> anyhow::Result<SystemTime> {
    let updated = git_stdout(repopath, &["log", "--format=%aI", "-n", "1"])?;
    let updated = updated.trim();
    if updated.is_empty() {
        bail!(
            "failed to determine latest git commit time for {}",
            repopath
        );
    }
    let timestamp = updated
        .parse::<Timestamp>()
        .with_context(|| format!("failed to parse git timestamp '{updated}' for {}", repopath))?;
    Ok(SystemTime::from(timestamp))
}

impl GitBackend for ExternalGitBackend {
    fn is_available(&self) -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    fn test_update_status(&self, repopath: &Utf8Path) -> UpdateStatus {
        if !repopath.join(".git").exists() {
            return UpdateStatus {
                needs_update: true,
                network_failure: false,
            };
        }

        let fetch = Command::new("git")
            .current_dir(repopath)
            .args(["fetch", "-q", "origin"])
            .output();
        let network_failure = fetch
            .as_ref()
            .ok()
            .and_then(|output| output.status.code())
            .is_some_and(|code| code == 128);

        let branch = match git_stdout(repopath, &["branch", "--show-current"]) {
            Ok(branch) if !branch.trim().is_empty() => branch,
            _ => {
                return UpdateStatus {
                    needs_update: true,
                    network_failure,
                };
            }
        };
        let branch = branch.trim();
        let range = format!("HEAD..origin/{branch}");
        let commits = git_stdout(repopath, &["log", &range, "--oneline"]).unwrap_or_default();

        UpdateStatus {
            needs_update: !commits.trim().is_empty(),
            network_failure,
        }
    }

    fn latest_author_for_path(&self, path: &Utf8Path) -> Option<String> {
        let repo_root = path
            .ancestors()
            .find(|candidate| candidate.join(".git").exists())?;
        let relative = path.strip_prefix(repo_root).ok()?;
        let output = Command::new("git")
            .current_dir(repo_root)
            .args(["log", "-1", "-s", "--format=%an", relative.as_str()])
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .filter(|value| !value.is_empty())
    }
}

fn git_stdout(repopath: &Utf8Path, args: &[&str]) -> anyhow::Result<String> {
    let output = git_output(repopath, args)?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn run_git(repopath: &Utf8Path, args: &[&str]) -> anyhow::Result<()> {
    let output = git_output(repopath, args)?;
    if !output.status.success() {
        bail!(
            "git {:?} failed in {}\nstdout: {}\nstderr: {}",
            args,
            repopath,
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn git_output(repopath: &Utf8Path, args: &[&str]) -> anyhow::Result<std::process::Output> {
    Command::new("git")
        .current_dir(repopath)
        .args(args)
        .output()
        .with_context(|| format!("failed to run git in {}", repopath))
}

#[cfg(test)]
mod tests {
    use std::{fs, process::Command};

    use camino::Utf8PathBuf;
    use tempfile::TempDir;

    use super::{GitBackend, system_backend};

    #[test]
    fn latest_author_tracks_last_commit_that_touched_file() {
        let fixture = GitFixture::new();
        fixture.commit_file("bucket\\demo.json", r#"{"version":"1.0.0"}"#, "Alice");
        fixture.commit_file("README.md", "seed", "Bob");
        fixture.commit_file("bucket\\demo.json", r#"{"version":"1.1.0"}"#, "Carol");

        let author = system_backend()
            .latest_author_for_path(&fixture.repo_root.join("bucket").join("demo.json"))
            .expect("author should resolve from git history");

        assert_eq!(author, "Carol");
    }

    struct GitFixture {
        _temp: TempDir,
        repo_root: Utf8PathBuf,
    }

    impl GitFixture {
        fn new() -> Self {
            let temp = TempDir::new().expect("temp dir should be created");
            let repo_root = Utf8PathBuf::from_path_buf(temp.path().join("repo"))
                .expect("temp path should be valid UTF-8");
            fs::create_dir_all(&repo_root).expect("repo root should exist");

            run_git(&repo_root, &["init"]);
            run_git(&repo_root, &["config", "commit.gpgsign", "false"]);
            run_git(
                &repo_root,
                &["config", "user.email", "codex@example.invalid"],
            );

            Self {
                _temp: temp,
                repo_root,
            }
        }

        fn commit_file(&self, relative_path: &str, content: &str, author: &str) {
            let path = self.repo_root.join(relative_path);
            fs::create_dir_all(path.parent().expect("fixture file should have a parent"))
                .expect("fixture parent should exist");
            fs::write(&path, content).expect("fixture file should be written");

            run_git(&self.repo_root, &["add", relative_path]);
            run_git(&self.repo_root, &["config", "user.name", author]);
            run_git(
                &self.repo_root,
                &[
                    "commit",
                    "--author",
                    &format!("{author} <{author}@example.invalid>"),
                    "-m",
                    author,
                ],
            );
        }
    }

    fn run_git(cwd: &Utf8PathBuf, args: &[&str]) {
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
}
