use anyhow::{Result, anyhow};
use reqwest::{Client, RequestBuilder};
use serde::Deserialize;

// ─── Router-level /v1/models ─────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct RouterModel {
    pub id: String,
    pub status: RouterModelStatus,
    /// Model metadata (n_params, size, …) — only present while the model is loaded
    #[serde(default)]
    pub meta: Option<ModelMeta>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RouterModelStatus {
    pub value: String, // "loaded" | "unloaded" | "loading" | "unloading"
    pub args: Vec<String>,
}

impl RouterModel {
    /// Extract the backend port from the args list: `--port 49341` (display only)
    pub fn port(&self) -> Option<u16> {
        let mut it = self.status.args.iter();
        while let Some(arg) = it.next() {
            if arg == "--port"
                && let Some(p) = it.next()
                && let Ok(n) = p.parse::<u16>()
                && n > 0
            {
                return Some(n);
            }
        }
        None
    }

    pub fn is_loaded(&self) -> bool {
        self.status.value == "loaded"
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RouterModelsResponse {
    pub data: Vec<RouterModel>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelMeta {
    pub n_params: Option<u64>,
    pub size: Option<u64>, // bytes
    pub n_ctx_train: Option<u64>,
    #[allow(dead_code)]
    pub n_vocab: Option<u32>,
    #[allow(dead_code)]
    pub n_embd: Option<u32>,
}

// ─── /slots (proxied by the router via ?model=<id>) ──────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct SlotNextToken {
    pub n_decoded: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Slot {
    pub id: u32,
    pub is_processing: bool,
    pub id_task: Option<i64>,
    pub next_token: Option<Vec<SlotNextToken>>,
}

impl Slot {
    pub fn n_decoded(&self) -> u64 {
        self.next_token
            .as_ref()
            .and_then(|v| v.first())
            .map(|t| t.n_decoded)
            .unwrap_or(0)
    }
}

// ─── Aggregated fetch result ──────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct LoadedModelData {
    pub model_id: String,
    pub port: Option<u16>,
    pub meta: Option<ModelMeta>,
    pub slots: Vec<Slot>,
}

#[derive(Debug, Clone)]
pub struct FetchResult {
    pub all_models: Vec<RouterModel>,
    pub loaded: Vec<LoadedModelData>,
    pub error: Option<String>,
}

pub async fn fetch_all(client: &Client, base_url: &str, api_key: &str) -> FetchResult {
    let mut result = FetchResult {
        all_models: vec![],
        loaded: vec![],
        error: None,
    };

    // 1. Fetch the router model list (loaded entries already carry their meta)
    let models = match fetch_router_models(client, base_url, api_key).await {
        Ok(m) => m,
        Err(e) => {
            result.error = Some(format!("Router: {e}"));
            return result;
        }
    };

    result.all_models = models.clone();

    // 2. Fetch slots for each loaded model in parallel (via the router)
    let loaded: Vec<RouterModel> = models.into_iter().filter(|m| m.is_loaded()).collect();

    let mut handles = vec![];
    for model in loaded {
        let client = client.clone();
        let base_url = base_url.to_string();
        let api_key = api_key.to_string();
        let model_id = model.id.clone();
        handles.push(tokio::spawn(async move {
            (
                model,
                fetch_slots(&client, &base_url, &model_id, &api_key).await,
            )
        }));
    }

    for handle in handles {
        match handle.await {
            Ok((model, Ok(slots))) => {
                let port = model.port();
                result.loaded.push(LoadedModelData {
                    model_id: model.id,
                    port,
                    meta: model.meta,
                    slots,
                });
            }
            Ok((model, Err(e))) => {
                // Non-fatal: keep the model (its meta is still useful) and report the failure
                if result.error.is_none() {
                    result.error = Some(format!("{}: {e}", model.id));
                }
                let port = model.port();
                result.loaded.push(LoadedModelData {
                    model_id: model.id,
                    port,
                    meta: model.meta,
                    slots: vec![],
                });
            }
            Err(e) => {
                if result.error.is_none() {
                    result.error = Some(format!("slots task failed: {e}"));
                }
            }
        }
    }

    // Sort loaded models by id for stable display
    result.loaded.sort_by(|a, b| a.model_id.cmp(&b.model_id));

    result
}

async fn fetch_router_models(
    client: &Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<RouterModel>> {
    let resp: RouterModelsResponse =
        with_auth(client.get(format!("{base_url}/v1/models")), api_key)
            .send()
            .await?
            .json()
            .await?;
    Ok(resp.data)
}

async fn fetch_slots(
    client: &Client,
    base_url: &str,
    model_id: &str,
    api_key: &str,
) -> Result<Vec<Slot>> {
    let resp = with_auth(
        client
            .get(format!("{base_url}/slots"))
            .query(&[("model", model_id)]),
        api_key,
    )
    .send()
    .await?;

    if !resp.status().is_success() {
        return Err(anyhow!("slots returned {}", resp.status()));
    }

    Ok(resp.json().await?)
}

/// Attach the API key only when one is configured (some servers reject unexpected auth headers)
fn with_auth(builder: RequestBuilder, api_key: &str) -> RequestBuilder {
    if api_key.is_empty() {
        builder
    } else {
        builder.bearer_auth(api_key)
    }
}
