use anyhow::{anyhow, Result};
use async_trait::async_trait;
use futures::StreamExt;
use language::{LanguageServerName, LspAdapter, LspAdapterDelegate};
use lsp::LanguageServerBinary;
use node_runtime::NodeRuntime;
use serde_json::json;
use smol::fs;
use std::{
    any::Any,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
};
use util::ResultExt;

const SERVER_PATH: &'static str = "node_modules/@astrojs/language-server/bin/nodeServer.js";

fn server_binary_arguments(server_path: &Path) -> Vec<OsString> {
    vec![server_path.into(), "--stdio".into()]
}

pub struct AstroLspAdapter {
    node: Arc<dyn NodeRuntime>,
}

impl AstroLspAdapter {
    pub fn new(node: Arc<dyn NodeRuntime>) -> Self {
        AstroLspAdapter { node }
    }
}

#[async_trait]
impl LspAdapter for AstroLspAdapter {
    fn name(&self) -> LanguageServerName {
        LanguageServerName("astro-language-server".into())
    }

    fn short_name(&self) -> &'static str {
        "astro"
    }

    async fn fetch_latest_server_version(
        &self,
        _: &dyn LspAdapterDelegate,
    ) -> Result<Box<dyn 'static + Any + Send>> {
        Ok(Box::new(
            self.node
                .npm_package_latest_version("@astrojs/language-server")
                .await?,
        ) as Box<_>)
    }

    async fn fetch_server_binary(
        &self,
        version: Box<dyn 'static + Send + Any>,
        container_dir: PathBuf,
        _: &dyn LspAdapterDelegate,
    ) -> Result<LanguageServerBinary> {
        let version = version.downcast::<String>().unwrap();
        let server_path = container_dir.join(SERVER_PATH);

        if fs::metadata(&server_path).await.is_err() {
            self.node
                .npm_install_packages(
                    &container_dir,
                    &[("@astrojs/language-server", version.as_str())],
                )
                .await?;
        }

        Ok(LanguageServerBinary {
            path: self.node.binary_path().await?,
            arguments: server_binary_arguments(&server_path),
        })
    }

    async fn cached_server_binary(
        &self,
        container_dir: PathBuf,
        _: &dyn LspAdapterDelegate,
    ) -> Option<LanguageServerBinary> {
        get_cached_server_binary(container_dir, &*self.node).await
    }

    async fn installation_test_binary(
        &self,
        container_dir: PathBuf,
    ) -> Option<LanguageServerBinary> {
        get_cached_server_binary(container_dir, &*self.node).await
    }

    fn initialization_options(&self) -> Option<serde_json::Value> {
        Some(json!({
            "provideFormatter": true,
            "typescript": {
                "tsdk": "node_modules/typescript/lib",
            }
        }))
    }

    fn prettier_plugins(&self) -> &[&'static str] {
        &["prettier-plugin-astro"]
    }
}

async fn get_cached_server_binary(
    container_dir: PathBuf,
    node: &dyn NodeRuntime,
) -> Option<LanguageServerBinary> {
    (|| async move {
        let mut last_version_dir = None;
        let mut entries = fs::read_dir(&container_dir).await?;
        while let Some(entry) = entries.next().await {
            let entry = entry?;
            if entry.file_type().await?.is_dir() {
                last_version_dir = Some(entry.path());
            }
        }
        let last_version_dir = last_version_dir.ok_or_else(|| anyhow!("no cached binary"))?;
        let server_path = last_version_dir.join(SERVER_PATH);
        if server_path.exists() {
            Ok(LanguageServerBinary {
                path: node.binary_path().await?,
                arguments: server_binary_arguments(&server_path),
            })
        } else {
            Err(anyhow!(
                "missing executable in directory {:?}",
                last_version_dir
            ))
        }
    })()
    .await
    .log_err()
}