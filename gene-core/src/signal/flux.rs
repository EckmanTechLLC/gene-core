use crate::signal::bus::SignalBus;
use crate::signal::types::SignalId;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Well-known Flux earthquake signal IDs — parallel to WorldSignalIds.
pub struct FluxSignalIds {
    pub quake_rate:      SignalId,
    pub quake_magnitude: SignalId,
    pub quake_depth:     SignalId,
    pub quake_sig:       SignalId,
}

/// Accumulated state for a single earthquake entity.
/// Properties arrive one at a time via state_update messages.
#[derive(Default, Clone)]
struct EarthquakeEntity {
    magnitude:    f64,
    depth_km:     f64,
    sig:          f64,
    event_time_s: f64,   // "time" property / 1000.0  (USGS ms → seconds)
}

/// Shared state between the WebSocket background task and FluxPoller.
pub struct FluxEarthquakeState {
    pub entities:  HashMap<String, EarthquakeEntity>,
    pub connected: bool,
}

impl FluxEarthquakeState {
    pub fn new() -> Self {
        Self { entities: HashMap::new(), connected: false }
    }
}

impl Default for FluxEarthquakeState {
    fn default() -> Self { Self::new() }
}

/// Reads aggregated earthquake state and writes four derived signals to the bus.
/// Called every 10 ticks in the main loop, following the WorldSignalPoller pattern.
pub struct FluxPoller {
    state:       Arc<Mutex<FluxEarthquakeState>>,
    initialized: bool,
}

impl FluxPoller {
    pub fn new(state: Arc<Mutex<FluxEarthquakeState>>) -> Self {
        Self { state, initialized: false }
    }

    /// Update earthquake signals on the bus. Should be called every 10 ticks.
    pub fn poll(&mut self, bus: &mut SignalBus, ids: &FluxSignalIds) {
        // First call: establish baseline, do not emit values
        if !self.initialized {
            self.initialized = true;
            return;
        }

        // Non-blocking lock — if the WS task is mid-write, skip this poll
        let Ok(state) = self.state.try_lock() else { return };

        // No data yet and not connected — let signals decay naturally
        if !state.connected && state.entities.is_empty() {
            return;
        }

        let now_s = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        let window_start = now_s - 3600.0; // 1-hour rolling window

        // Collect recent events: quake must have occurred in the last hour
        // and must have a valid magnitude (non-zero means the property was received)
        let recent: Vec<&EarthquakeEntity> = state.entities.values()
            .filter(|e| e.event_time_s >= window_start && e.magnitude > 0.0)
            .collect();

        let count = recent.len() as f64;

        // Ceiling: 20 events/hr is an active period globally for M2.5+
        let raw_rate = (count / 20.0).clamp(0.0, 1.0);

        let raw_mag = if count > 0.0 {
            recent.iter().map(|e| e.magnitude).fold(0.0f64, f64::max) / 9.0
        } else {
            0.0
        };

        // Inverted depth: shallow quakes (low depth_km) = high signal value
        let raw_depth = if count > 0.0 {
            let mean_depth = recent.iter().map(|e| e.depth_km).sum::<f64>() / count;
            (1.0 - mean_depth / 700.0).clamp(0.0, 1.0)
        } else {
            0.5 // neutral when no data
        };

        // USGS significance: 0–2000 scale
        let raw_sig = if count > 0.0 {
            recent.iter().map(|e| e.sig).fold(0.0f64, f64::max) / 2000.0
        } else {
            0.0
        };

        // Release lock before touching bus — no lock held during set_value calls
        drop(state);

        // 95/5 EMA — slow smoothing, seismic activity changes on hour timescales
        let ema = |cur: f64, raw: f64| (cur * 0.95 + raw * 0.05).clamp(0.0, 1.0);

        let cur = bus.get_value(ids.quake_rate);
        bus.set_value(ids.quake_rate, ema(cur, raw_rate));

        let cur = bus.get_value(ids.quake_magnitude);
        bus.set_value(ids.quake_magnitude, ema(cur, raw_mag));

        let cur = bus.get_value(ids.quake_depth);
        bus.set_value(ids.quake_depth, ema(cur, raw_depth));

        let cur = bus.get_value(ids.quake_sig);
        bus.set_value(ids.quake_sig, ema(cur, raw_sig));
    }
}

/// Spawn the background WebSocket task on a dedicated OS thread with its own
/// tokio runtime. This isolates the WS connection from gene's tick loop, which
/// is a tight synchronous loop that monopolizes a tokio worker thread and can
/// starve async TLS handshakes on resource-constrained VMs.
pub fn spawn_flux_ws_task(
    flux_url:  String,
    state:     Arc<Mutex<FluxEarthquakeState>>,
) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("flux ws tokio runtime");
        rt.block_on(run_flux_ws(flux_url, state));
    });
}

// ── Background WebSocket task ─────────────────────────────────────────────────

async fn run_flux_ws(url: String, state: Arc<Mutex<FluxEarthquakeState>>) {
    loop {
        tracing::info!("flux ws: connecting to {}", url);
        match connect_and_listen(url.clone(), state.clone()).await {
            Ok(()) => {
                tracing::info!("flux ws: connection closed cleanly");
            }
            Err(e) => {
                tracing::warn!("flux ws: error: {}", e);
            }
        }
        if let Ok(mut s) = state.lock() {
            s.connected = false;
        }
        tracing::info!("flux ws: reconnecting in 10s");
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}

async fn connect_and_listen(
    url:   String,
    state: Arc<Mutex<FluxEarthquakeState>>,
) -> anyhow::Result<()> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await?;

    // Subscribe to all entities; we filter client-side for flux-earthquakes/
    ws.send(Message::Text(
        r#"{"type":"subscribe","entity_id":"*"}"#.to_string().into()
    )).await?;

    if let Ok(mut s) = state.lock() {
        s.connected = true;
    }
    tracing::info!("flux ws: connected, subscribed to all entities");

    while let Some(msg) = ws.next().await {
        match msg? {
            Message::Text(text) => {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                    handle_message(&json, &state);
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    Ok(())
}

fn handle_message(
    json:  &serde_json::Value,
    state: &Arc<Mutex<FluxEarthquakeState>>,
) {
    let msg_type = match json.get("type").and_then(|v| v.as_str()) {
        Some(t) => t,
        None    => return,
    };

    match msg_type {
        "state_update" => {
            // Filter to flux-earthquakes namespace only
            let entity_id = match json.get("entity_id").and_then(|v| v.as_str()) {
                Some(id) if id.starts_with("flux-earthquakes/") => id.to_string(),
                _ => return,
            };
            let property = match json.get("property").and_then(|v| v.as_str()) {
                Some(p) => p,
                None    => return,
            };
            let value = match json.get("value") {
                Some(v) => v,
                None    => return,
            };

            if let Ok(mut s) = state.lock() {
                let entity = s.entities.entry(entity_id).or_default();
                match property {
                    "magnitude" => { entity.magnitude    = value.as_f64().unwrap_or(0.0); }
                    "depth_km"  => { entity.depth_km     = value.as_f64().unwrap_or(0.0); }
                    "sig"       => { entity.sig           = value.as_f64().unwrap_or(0.0); }
                    "time"      => {
                        // USGS time is milliseconds since epoch — convert to seconds
                        entity.event_time_s = value.as_f64().unwrap_or(0.0) / 1000.0;
                    }
                    _ => {} // lat, lon, tsunami, alert, place, url, etc. ignored
                }
            }
        }
        "entity_deleted" => {
            let entity_id = match json.get("entity_id").and_then(|v| v.as_str()) {
                Some(id) if id.starts_with("flux-earthquakes/") => id,
                _ => return,
            };
            if let Ok(mut s) = state.lock() {
                s.entities.remove(entity_id);
            }
        }
        _ => {} // metrics_update and unknown types silently ignored
    }
}
