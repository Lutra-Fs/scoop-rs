use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScoopPaths {
    root: Utf8PathBuf,
}

impl ScoopPaths {
    pub fn new(root: Utf8PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Utf8Path {
        &self.root
    }

    pub fn apps(&self) -> Utf8PathBuf {
        self.root.join("apps")
    }

    pub fn app_dir(&self, app: &str) -> Utf8PathBuf {
        self.apps().join(app)
    }

    pub fn current_dir(&self, app: &str) -> Utf8PathBuf {
        self.app_dir(app).join("current")
    }

    pub fn version_dir(&self, app: &str, version: &str) -> Utf8PathBuf {
        self.app_dir(app).join(version)
    }

    pub fn buckets(&self) -> Utf8PathBuf {
        self.root.join("buckets")
    }

    pub fn cache(&self) -> Utf8PathBuf {
        self.root.join("cache")
    }

    pub fn persist(&self) -> Utf8PathBuf {
        self.root.join("persist")
    }

    pub fn workspace(&self) -> Utf8PathBuf {
        self.root.join("workspace")
    }

    pub fn shims(&self) -> Utf8PathBuf {
        self.root.join("shims")
    }
}

#[cfg(test)]
mod tests {
    use camino::Utf8PathBuf;

    use super::ScoopPaths;

    #[test]
    fn joins_default_layout_consistently() {
        let paths = ScoopPaths::new(Utf8PathBuf::from("D:/Applications/Scoop"));

        assert_eq!(paths.apps(), "D:/Applications/Scoop/apps");
        assert_eq!(paths.app_dir("git"), "D:/Applications/Scoop/apps/git");
        assert_eq!(paths.buckets(), "D:/Applications/Scoop/buckets");
        assert_eq!(paths.cache(), "D:/Applications/Scoop/cache");
        assert_eq!(
            paths.current_dir("git"),
            "D:/Applications/Scoop/apps/git/current"
        );
        assert_eq!(paths.persist(), "D:/Applications/Scoop/persist");
        assert_eq!(paths.shims(), "D:/Applications/Scoop/shims");
        assert_eq!(
            paths.version_dir("git", "2.53.0.2"),
            "D:/Applications/Scoop/apps/git/2.53.0.2"
        );
        assert_eq!(paths.workspace(), "D:/Applications/Scoop/workspace");
    }
}
