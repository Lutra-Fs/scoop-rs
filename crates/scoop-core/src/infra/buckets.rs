use std::fs;

use anyhow::Context;
use camino::Utf8PathBuf;
use serde_json::Value;

use crate::RuntimeConfig;

pub fn bucket_root(config: &RuntimeConfig, name: Option<&str>) -> Utf8PathBuf {
    config
        .paths()
        .buckets()
        .join(name.filter(|name| !name.is_empty()).unwrap_or("main"))
}

pub fn bucket_manifests_dir(config: &RuntimeConfig, name: Option<&str>) -> Utf8PathBuf {
    let root = bucket_root(config, name);
    let manifests = root.join("bucket");
    if manifests.exists() { manifests } else { root }
}

pub fn local_bucket_names(config: &RuntimeConfig) -> anyhow::Result<Vec<String>> {
    let buckets_root = config.paths().buckets();
    if !buckets_root.exists() {
        return Ok(Vec::new());
    }

    let mut names = fs::read_dir(&buckets_root)
        .with_context(|| format!("failed to read buckets directory {}", buckets_root))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|kind| kind.is_dir())
                .and_then(|_| entry.file_name().into_string().ok())
        })
        .collect::<Vec<_>>();
    if names.is_empty() {
        return Ok(names);
    }

    let known = known_bucket_repos(config)?;
    let mut ordered = Vec::with_capacity(names.len());
    for (name, _) in &known {
        if let Some(index) = names.iter().position(|candidate| candidate == name) {
            ordered.push(names.remove(index));
        }
    }
    names.sort_unstable();
    ordered.extend(names);
    Ok(ordered)
}

pub fn known_bucket_repos(config: &RuntimeConfig) -> anyhow::Result<Vec<(String, String)>> {
    let path = config.paths().current_dir("scoop").join("buckets.json");
    if !path.is_file() {
        return Ok(Vec::new());
    }

    let source = fs::read_to_string(&path)
        .with_context(|| format!("failed to read known buckets {}", path))?;
    let value: Value = serde_json::from_str(&source)
        .with_context(|| format!("failed to parse known buckets {}", path))?;
    let Value::Object(entries) = value else {
        return Ok(Vec::new());
    };

    Ok(entries
        .into_iter()
        .filter_map(|(name, value)| value.as_str().map(|repo| (name, repo.to_owned())))
        .collect())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use camino::Utf8PathBuf;
    use tempfile::TempDir;

    use crate::RuntimeConfig;

    use super::{known_bucket_repos, local_bucket_names};

    #[test]
    fn orders_local_buckets_with_known_buckets_first() {
        let fixture = Fixture::new();
        fixture.write(
            "local",
            "apps\\scoop\\current\\buckets.json",
            r#"{"main":"https://github.com/ScoopInstaller/Main","extras":"https://github.com/ScoopInstaller/Extras"}"#,
        );
        fixture.mkdir("local", "buckets\\personal");
        fixture.mkdir("local", "buckets\\extras");
        fixture.mkdir("local", "buckets\\main");

        let names = local_bucket_names(&fixture.config()).expect("bucket names should load");
        assert_eq!(names, vec!["main", "extras", "personal"]);
    }

    #[test]
    fn reads_known_bucket_repositories_in_file_order() {
        let fixture = Fixture::new();
        fixture.write(
            "local",
            "apps\\scoop\\current\\buckets.json",
            r#"{"main":"https://github.com/ScoopInstaller/Main","versions":"https://github.com/ScoopInstaller/Versions"}"#,
        );

        let repos = known_bucket_repos(&fixture.config()).expect("known buckets should load");
        assert_eq!(
            repos,
            vec![
                (
                    String::from("main"),
                    String::from("https://github.com/ScoopInstaller/Main")
                ),
                (
                    String::from("versions"),
                    String::from("https://github.com/ScoopInstaller/Versions")
                ),
            ]
        );
    }

    struct Fixture {
        _temp: TempDir,
        local_root: Utf8PathBuf,
        global_root: Utf8PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = TempDir::new().expect("temp dir should be created");
            let root = Utf8PathBuf::from_path_buf(temp.path().to_path_buf())
                .expect("temp path should be valid UTF-8");
            let local_root = root.join("local");
            let global_root = root.join("global");
            fs::create_dir_all(&local_root).expect("local root should exist");
            fs::create_dir_all(&global_root).expect("global root should exist");

            Self {
                _temp: temp,
                local_root,
                global_root,
            }
        }

        fn config(&self) -> RuntimeConfig {
            RuntimeConfig::new(self.local_root.clone(), self.global_root.clone())
        }

        fn mkdir(&self, scope: &str, relative_path: &str) {
            let root = match scope {
                "local" => &self.local_root,
                "global" => &self.global_root,
                _ => panic!("unknown scope"),
            };
            fs::create_dir_all(root.join(relative_path)).expect("fixture directory should exist");
        }

        fn write(&self, scope: &str, relative_path: &str, content: &str) {
            let root = match scope {
                "local" => &self.local_root,
                "global" => &self.global_root,
                _ => panic!("unknown scope"),
            };
            let path = root.join(relative_path);
            fs::create_dir_all(path.parent().expect("fixture file should have a parent"))
                .expect("fixture parent should exist");
            fs::write(path, content).expect("fixture file should be written");
        }
    }
}
