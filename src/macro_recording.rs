use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MacroStep {
    pub timestamp_ms: u64,
    pub tool: String,
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RecordedMacro {
    pub name: Option<String>,
    pub started_at_ms: u64,
    pub stopped_at_ms: u64,
    pub steps: Vec<MacroStep>,
}

#[derive(Debug, Default, Clone)]
pub struct MacroRecorder {
    inner: Arc<Mutex<MacroRecorderState>>,
}

#[derive(Debug, Default)]
struct MacroRecorderState {
    recording: bool,
    name: Option<String>,
    started_at_ms: u64,
    steps: Vec<MacroStep>,
}

impl MacroRecorder {
    pub fn start(&self, name: Option<String>) -> String {
        let mut state = self.inner.lock().expect("macro recorder lock");
        state.recording = true;
        state.name = name;
        state.started_at_ms = now_ms();
        state.steps.clear();
        "Macro recording started. Actions routed through record-capable tools will be captured."
            .to_string()
    }

    pub fn stop(&self) -> RecordedMacro {
        let mut state = self.inner.lock().expect("macro recorder lock");
        state.recording = false;
        RecordedMacro {
            name: state.name.clone(),
            started_at_ms: state.started_at_ms,
            stopped_at_ms: now_ms(),
            steps: std::mem::take(&mut state.steps),
        }
    }

    pub fn is_recording(&self) -> bool {
        self.inner
            .lock()
            .map(|state| state.recording)
            .unwrap_or(false)
    }

    pub fn record_step(&self, tool: &str, params: serde_json::Value) {
        let Ok(mut state) = self.inner.lock() else {
            return;
        };
        if !state.recording {
            return;
        }
        state.steps.push(MacroStep {
            timestamp_ms: now_ms(),
            tool: tool.to_string(),
            params,
        });
    }

    pub fn export_skill_skeleton(macro_data: &RecordedMacro) -> String {
        let steps = macro_data
            .steps
            .iter()
            .map(|step| {
                format!(
                    "- Call `{}` with `{}`",
                    step.tool,
                    step.params.to_string().replace('\n', " ")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "# Recorded desktop workflow\n\nReplay the following Computer Use sequence:\n\n{steps}\n"
        )
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}