use crate::{
    RuntimeConfig,
    compat::catalog::{render_manifest_json, resolve_manifest},
};

pub fn render_manifest_for_app(
    config: &RuntimeConfig,
    app: &str,
) -> anyhow::Result<Option<String>> {
    match resolve_manifest(config, app)? {
        Some(manifest) => Ok(Some(render_manifest_json(&manifest.manifest)?)),
        None => Ok(None),
    }
}
