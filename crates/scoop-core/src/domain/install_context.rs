use camino::{Utf8Path, Utf8PathBuf};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookType {
    Installer,
    PreInstall,
    PostInstall,
    Uninstaller,
    PreUninstall,
    PostUninstall,
}

impl HookType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Installer => "installer",
            Self::PreInstall => "pre_install",
            Self::PostInstall => "post_install",
            Self::Uninstaller => "uninstaller",
            Self::PreUninstall => "pre_uninstall",
            Self::PostUninstall => "post_uninstall",
        }
    }
}

#[derive(Debug, Clone)]
pub struct InstallContext {
    app: String,
    version: String,
    architecture: String,
    global: bool,
    dir: Utf8PathBuf,
    original_dir: Utf8PathBuf,
    persist_dir: Utf8PathBuf,
    manifest: Value,
}

#[derive(Debug, Clone)]
pub struct InstallContextPaths {
    pub dir: Utf8PathBuf,
    pub original_dir: Utf8PathBuf,
    pub persist_dir: Utf8PathBuf,
}

impl InstallContext {
    pub fn new(
        app: String,
        version: String,
        architecture: String,
        global: bool,
        paths: InstallContextPaths,
        manifest: Value,
    ) -> Self {
        Self {
            app,
            version,
            architecture,
            global,
            dir: paths.dir,
            original_dir: paths.original_dir,
            persist_dir: paths.persist_dir,
            manifest,
        }
    }

    pub fn app(&self) -> &str {
        &self.app
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn architecture(&self) -> &str {
        &self.architecture
    }

    pub fn global(&self) -> bool {
        self.global
    }

    pub fn dir(&self) -> &Utf8Path {
        &self.dir
    }

    pub fn original_dir(&self) -> &Utf8Path {
        &self.original_dir
    }

    pub fn persist_dir(&self) -> &Utf8Path {
        &self.persist_dir
    }

    pub fn manifest(&self) -> &Value {
        &self.manifest
    }

    pub fn manifest_json(&self) -> anyhow::Result<String> {
        serde_json::to_string(&self.manifest).context("failed to serialize install manifest")
    }

    pub fn token_substitutions(&self) -> [(&'static str, String); 6] {
        [
            ("$original_dir", windows_path(&self.original_dir)),
            ("$persist_dir", windows_path(&self.persist_dir)),
            ("$architecture", self.architecture.clone()),
            ("$version", self.version.clone()),
            ("$global", self.global.to_string()),
            ("$dir", windows_path(&self.dir)),
        ]
    }

    pub fn substitute(&self, value: &str) -> String {
        let mut substituted = value.to_owned();
        for (token, replacement) in self.token_substitutions() {
            substituted = substituted.replace(token, &replacement);
        }
        substituted
    }
}

fn windows_path(path: &Utf8Path) -> String {
    path.as_str().replace('/', "\\")
}

use anyhow::Context;

#[cfg(test)]
mod tests {
    use camino::Utf8PathBuf;
    use serde_json::json;

    use super::{InstallContext, InstallContextPaths};

    #[test]
    fn substitutes_longest_tokens_first_like_upstream() {
        let context = InstallContext::new(
            String::from("demo"),
            String::from("1.2.3"),
            String::from("64bit"),
            false,
            InstallContextPaths {
                dir: Utf8PathBuf::from("D:/Applications/Scoop/apps/demo/current"),
                original_dir: Utf8PathBuf::from("D:/Applications/Scoop/apps/demo/1.2.3"),
                persist_dir: Utf8PathBuf::from("D:/Applications/Scoop/persist/demo"),
            },
            json!({ "version": "1.2.3" }),
        );

        assert_eq!(
            context.substitute("$original_dir|$persist_dir|$dir|$version|$global"),
            "D:\\Applications\\Scoop\\apps\\demo\\1.2.3|D:\\Applications\\Scoop\\persist\\demo|D:\\Applications\\Scoop\\apps\\demo\\current|1.2.3|false"
        );
    }
}
