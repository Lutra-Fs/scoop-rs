use std::fs;

use anyhow::Context;
use camino::Utf8PathBuf;

use super::windows::shortcut::create_shortcut_file;

const STARTMENU_OVERRIDE: &str = "SCOOP_RS_STARTMENU_ROOT";

pub fn shortcut_root(global: bool) -> anyhow::Result<Utf8PathBuf> {
    if let Some(value) = std::env::var_os(STARTMENU_OVERRIDE) {
        let root = Utf8PathBuf::from_path_buf(value.into())
            .map_err(|_| anyhow::anyhow!("{STARTMENU_OVERRIDE} should be valid UTF-8"))?;
        return Ok(root);
    }

    let base = if global {
        std::env::var("ProgramData")
            .map(Utf8PathBuf::from)
            .unwrap_or_else(|_| Utf8PathBuf::from("C:/ProgramData"))
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
    } else {
        std::env::var("APPDATA")
            .map(Utf8PathBuf::from)
            .unwrap_or_else(|_| Utf8PathBuf::from("C:/Users/Default/AppData/Roaming"))
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
    };
    Ok(base.join("Scoop Apps"))
}

pub fn create_shortcut(
    target: &str,
    shortcut_name: &str,
    arguments: &str,
    icon: Option<&str>,
    global: bool,
) -> anyhow::Result<Utf8PathBuf> {
    let root = shortcut_root(global)?;
    if let Some(parent) = root.join(shortcut_name).parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create shortcut parent {}", parent))?;
    }
    fs::create_dir_all(&root)
        .with_context(|| format!("failed to create shortcut root {}", root))?;
    let shortcut = root.join(format!("{shortcut_name}.lnk"));
    create_shortcut_file(target, &shortcut, arguments, icon)?;
    Ok(shortcut)
}
