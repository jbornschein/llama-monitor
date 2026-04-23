use anyhow::{Result, anyhow};
use reqwest::Client;
use serde::Deserialize;

// ─── Router-level /v1/models ─────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct RouterModel {
    pub id: String,
    pub status: RouterModelStatus,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RouterModelStatus {
    pub value: String, // "loaded" | "unloaded" | "loading" | "unloading"
    pub args: Vec<String>,
}

impl RouterModel {
    /// Extract the port from the args list: `--port 49341`
    pub fn port(&self) -> Option<u16> {
        let mut it = self.status.args.iter();
        while let Some(arg) = it.next() {
            if arg == "--port" {
                if let Some(p) = it.next() {
                    if let Ok(n) = p.parse::<u16>() {
                        if n > 0 {
                            return Some(n);
                        }
                    }
                }
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

// ─── Per-model /v1/models (metadata) ─────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ModelMeta {
    pub n_params: Option<u64>,
    pub size: Option<u64>,      // bytes
    pub n_ctx_train: Option<u64>,
    #[allow(dead_code)]
    pub n_vocab: Option<u32>,
    #[allow(dead_code)]
    pub n_embd: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PerModelEntry {
    #[allow(dead_code)]
    pub id: String,
    pub meta: Option<ModelMeta>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PerModelResponse {
    pub data: Vec<PerModelEntry>,
}

// ─── /slots ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct SlotNextToken {
    pub n_decoded: u64,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct SlotParams {
    pub chat_format: Option<String>,
    pub temperature: Option<f64>,
    pub top_k: Option<u32>,
    pub top_p: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Slot {
    pub id: u32,
    pub is_processing: bool,
    pub id_task: Option<i64>,
    #[allow(dead_code)]
    pub params: Option<SlotParams>,
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
    pub port: u16,
    pub meta: Option<ModelMeta>,
    pub slots: Vec<Slot>,
    #[allow(dead_code)]
    pub fetch_time: std::time::Instant,
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

    // 1. Fetch router model list
    let models = match fetch_router_models(client, base_url, api_key).await {
        Ok(m) => m,
        Err(e) => {
            result.error = Some(format!("Router: {e}"));
            return result;
        }
    };

    result.all_models = models.clone();

    // 2. For each loaded model, fetch slots + metadata in parallel
    let loaded: Vec<RouterModel> = models.into_iter().filter(|m| m.is_loaded()).collect();

    let mut handles = vec![];
    for model in loaded {
        if let Some(port) = model.port() {
            let client = client.clone();
            let api_key = api_key.to_string();
            handles.push(tokio::spawn(async move {
                fetch_model_details(&client, &model.id, port, &api_key).await
            }));
        }
    }

    for handle in handles {
        match handle.await {
            Ok(Ok(data)) => result.loaded.push(data),
            Ok(Err(e)) => {
                // non-fatal: show partial data
                if result.error.is_none() {
                    result.error = Some(e.to_string());
                }
            }
            _ => {}
        }
    }

    // Sort loaded models by id for stable display
    result.loaded.sort_by(|a, b| a.model_id.cmp(&b.model_id));

    result
}

async fn fetch_router_models(client: &Client, base_url: &str, api_key: &str) -> Result<Vec<RouterModel>> {
    let resp: RouterModelsResponse = client
        .get(format!("{base_url}/v1/models"))
        .bearer_auth(api_key)
        .send()
        .await?
        .json()
        .await?;
    Ok(resp.data)
}

async fn fetch_model_details(client: &Client, model_id: &str, port: u16, api_key: &str) -> Result<LoadedModelData> {
    let base = format!("http://127.0.0.1:{port}");

    // Fetch slots and metadata concurrently
    let (slots_res, meta_res) = tokio::join!(
        fetch_slots(client, &base, api_key),
        fetch_model_meta(client, &base, api_key),
    );

    let slots = slots_res.unwrap_or_default();
    let meta = meta_res.ok().flatten();

    Ok(LoadedModelData {
        model_id: model_id.to_string(),
        port,
        meta,
        slots,
        fetch_time: std::time::Instant::now(),
    })
}

async fn fetch_slots(client: &Client, base: &str, api_key: &str) -> Result<Vec<Slot>> {
    let resp = client
        .get(format!("{base}/slots"))
        .bearer_auth(api_key)
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(anyhow!("slots returned {}", resp.status()));
    }

    Ok(resp.json().await?)
}

async fn fetch_model_meta(client: &Client, base: &str, api_key: &str) -> Result<Option<ModelMeta>> {
    let resp: PerModelResponse = client
        .get(format!("{base}/v1/models"))
        .bearer_auth(api_key)
        .send()
        .await?
        .json()
        .await?;

    Ok(resp.data.into_iter().next().and_then(|e| e.meta))
}
