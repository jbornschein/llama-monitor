use crate::api::{FetchResult, LoadedModelData, RouterModel};
use std::collections::HashMap;
use std::time::Instant;

const HISTORY_LEN: usize = 60;

#[derive(Debug, Clone)]
pub struct SlotHistory {
    /// Last n_decoded value seen for this slot
    pub last_n_decoded: u64,
    pub last_fetch_time: Instant,
    /// Tokens/sec history (last HISTORY_LEN samples)
    pub tps_history: Vec<f64>,
    pub current_tps: f64,
    /// id_task of the last request we observed
    last_id_task: Option<i64>,
    /// True while a slot is processing but hasn't yet decoded any tokens
    pub in_prefill: bool,
}

impl SlotHistory {
    fn new(n_decoded: u64, id_task: Option<i64>, now: Instant) -> Self {
        Self {
            last_n_decoded: n_decoded,
            last_fetch_time: now,
            tps_history: Vec::with_capacity(HISTORY_LEN),
            current_tps: 0.0,
            last_id_task: id_task,
            in_prefill: false,
        }
    }

    fn update(&mut self, n_decoded: u64, id_task: Option<i64>, is_processing: bool, now: Instant) {
        // Detect a new request starting on this slot
        if id_task.is_some() && id_task != self.last_id_task && is_processing {
            self.in_prefill = true;
            self.current_tps = 0.0;
        }
        // Once tokens start flowing, prefill is done
        if n_decoded > self.last_n_decoded {
            self.in_prefill = false;
        }
        if !is_processing {
            self.in_prefill = false;
        }

        let elapsed = now.duration_since(self.last_fetch_time).as_secs_f64();
        if elapsed > 0.1 && n_decoded >= self.last_n_decoded {
            let delta = n_decoded - self.last_n_decoded;
            self.current_tps = delta as f64 / elapsed;
        } else if n_decoded < self.last_n_decoded {
            // Counter reset for new request
            self.current_tps = 0.0;
        }
        self.tps_history.push(self.current_tps);
        if self.tps_history.len() > HISTORY_LEN {
            self.tps_history.remove(0);
        }
        self.last_n_decoded = n_decoded;
        self.last_id_task = id_task;
        self.last_fetch_time = now;
    }
}

/// Key: (model_id, slot_id)
type SlotKey = (String, u32);

pub struct App {
    pub server_url: String,
    pub all_models: Vec<RouterModel>,
    pub loaded_models: Vec<LoadedModelData>,
    pub slot_histories: HashMap<SlotKey, SlotHistory>,
    pub refreshing: bool,
    pub last_update: Option<Instant>,
    pub error: Option<String>,
    pub scroll: usize,
}

impl App {
    pub fn new(server_url: String) -> Self {
        Self {
            server_url,
            all_models: vec![],
            loaded_models: vec![],
            slot_histories: HashMap::new(),
            refreshing: false,
            last_update: None,
            error: None,
            scroll: 0,
        }
    }

    pub fn set_refreshing(&mut self, v: bool) {
        self.refreshing = v;
    }

    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    pub fn scroll_down(&mut self) {
        self.scroll += 1;
    }

    pub fn update(&mut self, result: FetchResult) {
        self.refreshing = false;
        self.last_update = Some(Instant::now());
        self.error = result.error.clone();
        self.all_models = result.all_models;

        let now = Instant::now();

        for model_data in &result.loaded {
            for slot in &model_data.slots {
                let key = (model_data.model_id.clone(), slot.id);
                let n_decoded = slot.n_decoded();

                match self.slot_histories.get_mut(&key) {
                    Some(hist) => hist.update(n_decoded, slot.id_task, slot.is_processing, now),
                    None => {
                        self.slot_histories
                            .insert(key, SlotHistory::new(n_decoded, slot.id_task, now));
                    }
                }
            }
        }

        // Remove histories for models no longer loaded
        let active_keys: std::collections::HashSet<SlotKey> = result
            .loaded
            .iter()
            .flat_map(|m| m.slots.iter().map(|s| (m.model_id.clone(), s.id)))
            .collect();
        self.slot_histories.retain(|k, _| active_keys.contains(k));

        self.loaded_models = result.loaded;
    }

    pub fn slot_in_prefill(&self, model_id: &str, slot_id: u32) -> bool {
        self.slot_histories
            .get(&(model_id.to_string(), slot_id))
            .map(|h| h.in_prefill)
            .unwrap_or(false)
    }

    pub fn slot_tps(&self, model_id: &str, slot_id: u32) -> f64 {
        self.slot_histories
            .get(&(model_id.to_string(), slot_id))
            .map(|h| h.current_tps)
            .unwrap_or(0.0)
    }

    #[allow(dead_code)]
    pub fn tps_history(&self, model_id: &str, slot_id: u32) -> &[f64] {
        self.slot_histories
            .get(&(model_id.to_string(), slot_id))
            .map(|h| h.tps_history.as_slice())
            .unwrap_or(&[])
    }

    /// Aggregate tps across all active slots for a model
    pub fn model_tps(&self, model_id: &str) -> f64 {
        self.slot_histories
            .iter()
            .filter(|((mid, _), _)| mid == model_id)
            .map(|(_, h)| h.current_tps)
            .sum()
    }

    /// Aggregate tps history across all slots of a model (summed per time step)
    pub fn model_tps_history(&self, model_id: &str) -> Vec<f64> {
        let histories: Vec<&Vec<f64>> = self
            .slot_histories
            .iter()
            .filter(|((mid, _), _)| mid == model_id)
            .map(|(_, h)| &h.tps_history)
            .collect();

        if histories.is_empty() {
            return vec![0.0; HISTORY_LEN];
        }

        let len = histories[0].len();
        let mut result = vec![0.0f64; len];
        for hist in histories {
            for (i, v) in hist.iter().enumerate() {
                if i < result.len() {
                    result[i] += v;
                }
            }
        }
        result
    }

    pub fn active_slot_count(&self, model_id: &str) -> usize {
        self.loaded_models
            .iter()
            .find(|m| m.model_id == model_id)
            .map(|m| m.slots.iter().filter(|s| s.is_processing).count())
            .unwrap_or(0)
    }

    pub fn total_slot_count(&self, model_id: &str) -> usize {
        self.loaded_models
            .iter()
            .find(|m| m.model_id == model_id)
            .map(|m| m.slots.len())
            .unwrap_or(0)
    }
}

/// Format bytes as human-readable
pub fn fmt_bytes(bytes: u64) -> String {
    const GB: f64 = 1_073_741_824.0;
    const MB: f64 = 1_048_576.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1}GB", b / GB)
    } else if b >= MB {
        format!("{:.0}MB", b / MB)
    } else {
        format!("{bytes}B")
    }
}

/// Format parameter count
pub fn fmt_params(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.1}B", n as f64 / 1e9)
    } else if n >= 1_000_000 {
        format!("{:.0}M", n as f64 / 1e6)
    } else {
        format!("{n}")
    }
}
