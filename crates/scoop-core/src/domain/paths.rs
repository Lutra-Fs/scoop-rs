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
        let paths = ScoopPaths::new(Utf8PathBuf::from("C:/Users/example/scoop"));

        assert_eq!(paths.apps(), "C:/Users/example/scoop/apps");
        assert_eq!(paths.app_dir("git"), "C:/Users/example/scoop/apps/git");
        assert_eq!(paths.buckets(), "C:/Users/example/scoop/buckets");
        assert_eq!(paths.cache(), "C:/Users/example/scoop/cache");
        assert_eq!(
            paths.current_dir("git"),
            "C:/Users/example/scoop/apps/git/current"
        );
        assert_eq!(paths.persist(), "C:/Users/example/scoop/persist");
        assert_eq!(paths.shims(), "C:/Users/example/scoop/shims");
        assert_eq!(
            paths.version_dir("git", "2.53.0.2"),
            "C:/Users/example/scoop/apps/git/2.53.0.2"
        );
        assert_eq!(paths.workspace(), "C:/Users/example/scoop/workspace");
    }
}
