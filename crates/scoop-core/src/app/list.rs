use crate::{
    InstalledApp, RuntimeConfig,
    infra::installed::{compile_query, discover_installed_apps, filter_installed_apps},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListReport {
    pub query: Option<String>,
    pub apps: Vec<InstalledApp>,
}

pub fn list_installed(config: &RuntimeConfig, query: Option<&str>) -> anyhow::Result<ListReport> {
    let compiled_query = query.map(compile_query).transpose()?;
    let apps = discover_installed_apps(config)?;
    let filtered = filter_installed_apps(&apps, compiled_query.as_ref())
        .into_iter()
        .cloned()
        .collect();

    Ok(ListReport {
        query: query.map(str::to_owned),
        apps: filtered,
    })
}
