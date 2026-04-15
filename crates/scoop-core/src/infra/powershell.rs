use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, bail};
use camino::{Utf8Path, Utf8PathBuf};

use crate::domain::install_context::{HookType, InstallContext};

pub fn run_install_hook(
    hook_type: HookType,
    script: &str,
    context: &InstallContext,
) -> anyhow::Result<()> {
    if script.trim().is_empty() {
        return Ok(());
    }

    let script_body = powershell_script(script, context)?;
    let output = run_powershell_script(&script_body)
        .with_context(|| format!("failed to launch PowerShell for {}", hook_type.as_str()))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !stderr.is_empty() {
        bail!("{} script failed: {}", hook_type.as_str(), stderr);
    }
    if !stdout.is_empty() {
        bail!("{} script failed: {}", hook_type.as_str(), stdout);
    }
    bail!("{} script failed", hook_type.as_str())
}

fn run_powershell_script(script_body: &str) -> anyhow::Result<std::process::Output> {
    let temp_dir = Utf8PathBuf::from_path_buf(std::env::temp_dir())
        .map_err(|_| anyhow::anyhow!("temporary directory path should be valid UTF-8"))?;
    fs::create_dir_all(&temp_dir)
        .with_context(|| format!("failed to create temporary directory {}", temp_dir))?;
    let script_path = temp_dir.join(temp_script_name("script"));
    fs::write(&script_path, script_body)
        .with_context(|| format!("failed to write hook script {}", script_path))?;

    let output = Command::new("pwsh")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            script_path.as_str(),
        ])
        .output()
        .with_context(|| format!("failed to launch PowerShell script {}", script_path))?;

    let _ = fs::remove_file(&script_path);
    Ok(output)
}

fn powershell_script(script: &str, context: &InstallContext) -> anyhow::Result<String> {
    let manifest_json = escape_single_quoted(&context.manifest_json()?);
    Ok(format!(
        concat!(
            "$dir = '{}'\r\n",
            "$original_dir = '{}'\r\n",
            "$persist_dir = '{}'\r\n",
            "$global = ${}\r\n",
            "$architecture = '{}'\r\n",
            "$version = '{}'\r\n",
            "$app = '{}'\r\n",
            "$manifest = ConvertFrom-Json -InputObject '{}'\r\n",
            "$hook_script = @'\r\n",
            "{}\r\n",
            "'@\r\n",
            "Invoke-Command ([scriptblock]::Create($hook_script))\r\n"
        ),
        escape_single_quoted(&windows_path(context.dir())),
        escape_single_quoted(&windows_path(context.original_dir())),
        escape_single_quoted(&windows_path(context.persist_dir())),
        if context.global() { "true" } else { "false" },
        escape_single_quoted(context.architecture()),
        escape_single_quoted(context.version()),
        escape_single_quoted(context.app()),
        manifest_json,
        script,
    ))
}

fn temp_script_name(prefix: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("scoop-rs-{prefix}-{now}.ps1")
}

fn escape_single_quoted(value: &str) -> String {
    value.replace('\'', "''")
}

fn windows_path(path: &Utf8Path) -> String {
    path.as_str().replace('/', "\\")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use camino::Utf8PathBuf;
    use serde_json::json;
    use tempfile::TempDir;

    use crate::domain::install_context::{HookType, InstallContext, InstallContextPaths};

    use super::run_install_hook;

    #[cfg(windows)]
    #[test]
    fn injects_install_variables_into_powershell_hooks() {
        let fixture = TempDir::new().expect("temp dir should exist");
        let root = Utf8PathBuf::from_path_buf(fixture.path().to_path_buf())
            .expect("temp path should be valid UTF-8");
        let dir = root.join("apps").join("demo").join("1.2.3");
        let persist_dir = root.join("persist").join("demo");
        fs::create_dir_all(&dir).expect("install dir should exist");
        fs::create_dir_all(&persist_dir).expect("persist dir should exist");
        let marker = dir.join("hook.txt");
        let context = InstallContext::new(
            String::from("demo"),
            String::from("1.2.3"),
            String::from("64bit"),
            false,
            InstallContextPaths {
                dir: dir.clone(),
                original_dir: dir.clone(),
                persist_dir: persist_dir.clone(),
            },
            json!({ "version": "1.2.3", "description": "demo app" }),
        );

        run_install_hook(
            HookType::PreInstall,
            "$value = $app + '|' + $version + '|' + $architecture + '|' + $global + '|' + $dir + '|' + $persist_dir + '|' + $manifest.description; Set-Content -Path (Join-Path $dir 'hook.txt') -Value $value",
            &context,
        )
        .expect("hook should succeed");

        let written = fs::read_to_string(&marker).expect("hook output should exist");
        assert_eq!(
            written.trim(),
            format!(
                "demo|1.2.3|64bit|False|{}|{}|demo app",
                dir.as_str().replace('/', "\\"),
                persist_dir.as_str().replace('/', "\\"),
            )
        );
    }
}
