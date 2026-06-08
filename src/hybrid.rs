use crate::atspi_tree::AccessibilityNode;
use schemars::JsonSchema;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InputStrategy {
    AccessibilityRef,
    AccessibilityAction,
    CoordinateClick,
    HybridFallback,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct HybridRecommendation {
    pub hybrid_mode_enabled: bool,
    pub tree_node_count: usize,
    pub actionable_node_count: usize,
    pub recommended_strategy: InputStrategy,
    pub explanation: String,
    pub fallback_chain: Vec<String>,
}

pub fn hybrid_mode_enabled() -> bool {
    std::env::var("COMPUTER_USE_LINUX_HYBRID")
        .ok()
        .or_else(|| std::env::var("CU_HYBRID").ok())
        .map(|value| matches!(value.trim(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

pub fn recommend_strategy(nodes: &[AccessibilityNode]) -> HybridRecommendation {
    let actionable_node_count = nodes
        .iter()
        .filter(|node| {
            node.bounds.is_some()
                || !node.actions.is_empty()
                || node.supports_editable_text
                || node.value.is_some()
        })
        .count();

    let hybrid_enabled = hybrid_mode_enabled();
    let (recommended_strategy, explanation) = if actionable_node_count >= 3 {
        (
            InputStrategy::AccessibilityRef,
            "Accessibility tree has actionable nodes; prefer @eN refs and semantic selectors.".to_string(),
        )
    } else if actionable_node_count > 0 && hybrid_enabled {
        (
            InputStrategy::HybridFallback,
            "Sparse accessibility tree with hybrid mode enabled: try @eN refs first, then coordinate clicks from a bounded screenshot.".to_string(),
        )
    } else if actionable_node_count > 0 {
        (
            InputStrategy::AccessibilityRef,
            "Limited actionable nodes; use precise role/name selectors and verify with get_app_state after each action.".to_string(),
        )
    } else if hybrid_enabled {
        (
            InputStrategy::CoordinateClick,
            "No actionable accessibility nodes; hybrid mode routes to screenshot coordinates via portal/ydotool.".to_string(),
        )
    } else {
        (
            InputStrategy::HybridFallback,
            "No actionable nodes detected. Enable hybrid mode (COMPUTER_USE_LINUX_HYBRID=1) or pass coordinates from screenshot metadata.".to_string(),
        )
    };

    HybridRecommendation {
        hybrid_mode_enabled: hybrid_enabled,
        tree_node_count: nodes.len(),
        actionable_node_count,
        recommended_strategy,
        explanation,
        fallback_chain: vec![
            "AT-SPI element_index / semantic selector".to_string(),
            "AT-SPI primary action (perform_action)".to_string(),
            "uinput absolute pointer coordinate click".to_string(),
            "Wayland remote desktop portal".to_string(),
            "ydotool relative input".to_string(),
        ],
    }
}