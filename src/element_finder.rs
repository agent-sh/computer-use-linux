use crate::atspi_tree::AccessibilityNode;
use schemars::JsonSchema;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct FindElementMatch {
    pub element_index: u32,
    pub element_ref: String,
    pub role: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub score: f32,
    pub matched_fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct FindElementResult {
    pub description: String,
    pub matches: Vec<FindElementMatch>,
    pub best_match: Option<FindElementMatch>,
    pub strategy: String,
    pub explanation: String,
}

pub fn find_elements_by_description(
    nodes: &[AccessibilityNode],
    description: &str,
    limit: usize,
) -> FindElementResult {
    let query_tokens = tokenize(description);
    if query_tokens.is_empty() {
        return FindElementResult {
            description: description.to_string(),
            matches: Vec::new(),
            best_match: None,
            strategy: "natural_language_token_match".to_string(),
            explanation: "The description did not contain searchable tokens.".to_string(),
        };
    }

    let mut matches: Vec<FindElementMatch> = nodes
        .iter()
        .filter_map(|node| score_node(node, &query_tokens))
        .collect();
    matches.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.element_index.cmp(&right.element_index))
    });
    matches.truncate(limit.max(1));

    let best_match = matches.first().cloned();
    let explanation = if let Some(best) = &best_match {
        if best.score >= 0.75 {
            format!(
                "Matched @{} ({}) with high confidence via {}.",
                best.element_ref,
                best.role,
                best.matched_fields.join(", ")
            )
        } else if best.score >= 0.4 {
            format!(
                "Matched @{} ({}) with moderate confidence; verify with get_app_state before acting.",
                best.element_ref,
                best.role
            )
        } else {
            "Low-confidence match; consider hybrid coordinate fallback or a more specific description."
                .to_string()
        }
    } else {
        "No accessibility node matched the description. Use hybrid mode: screenshot + coordinates, or refine the query."
            .to_string()
    };

    FindElementResult {
        description: description.to_string(),
        matches,
        best_match,
        strategy: "natural_language_token_match".to_string(),
        explanation,
    }
}

fn score_node(node: &AccessibilityNode, query_tokens: &[String]) -> Option<FindElementMatch> {
    let role = normalize(&node.role);
    let name = node.name.as_deref().map(normalize).unwrap_or_default();
    let description = node
        .description
        .as_deref()
        .map(normalize)
        .unwrap_or_default();
    let text = node
        .text
        .as_ref()
        .and_then(|value| value.content.as_deref())
        .map(normalize)
        .unwrap_or_default();

    let mut matched_fields = Vec::new();
    let mut score = 0.0f32;
    for token in query_tokens {
        let mut token_score = 0.0f32;
        if name.contains(token) {
            token_score = token_score.max(1.0);
            if !matched_fields.contains(&"name".to_string()) {
                matched_fields.push("name".to_string());
            }
        }
        if description.contains(token) {
            token_score = token_score.max(0.85);
            if !matched_fields.contains(&"description".to_string()) {
                matched_fields.push("description".to_string());
            }
        }
        if text.contains(token) {
            token_score = token_score.max(0.8);
            if !matched_fields.contains(&"text".to_string()) {
                matched_fields.push("text".to_string());
            }
        }
        if role.contains(token) {
            token_score = token_score.max(0.7);
            if !matched_fields.contains(&"role".to_string()) {
                matched_fields.push("role".to_string());
            }
        }
        score += token_score;
    }

    if matched_fields.is_empty() {
        return None;
    }

    let normalized_score = score / query_tokens.len() as f32;
    let actionable_bonus = if node.bounds.is_some() || !node.actions.is_empty() {
        0.05
    } else {
        0.0
    };
    let showing_bonus = if node
        .states
        .iter()
        .any(|state| normalize(state) == "showing" || normalize(state) == "visible")
    {
        0.05
    } else {
        0.0
    };

    Some(FindElementMatch {
        element_index: node.index,
        element_ref: format!("@e{}", node.index),
        role: node.role.clone(),
        name: node.name.clone(),
        description: node.description.clone(),
        score: (normalized_score + actionable_bonus + showing_bonus).min(1.0),
        matched_fields,
    })
}

fn tokenize(description: &str) -> Vec<String> {
    description
        .split(|character: char| !character.is_alphanumeric())
        .map(normalize)
        .filter(|token| token.len() >= 2)
        .collect()
}

fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atspi_tree::{AccessibilityAction, Bounds};

    fn node(index: u32, role: &str, name: &str) -> AccessibilityNode {
        AccessibilityNode {
            index,
            parent_index: None,
            depth: 0,
            object_ref: format!("app:/node/{index}"),
            role: role.to_string(),
            name: Some(name.to_string()),
            description: None,
            child_count: 0,
            bounds: Some(Bounds {
                x: 0,
                y: 0,
                width: 10,
                height: 10,
            }),
            states: vec!["showing".to_string(), "visible".to_string()],
            actions: vec![AccessibilityAction {
                index: 0,
                name: "click".to_string(),
                description: String::new(),
                keybinding: String::new(),
            }],
            value: None,
            text: None,
            supports_editable_text: false,
        }
    }

    #[test]
    fn finds_save_button_by_natural_language() {
        let nodes = vec![
            node(1, "push button", "Cancel"),
            node(2, "push button", "Save"),
        ];
        let result = find_elements_by_description(&nodes, "the save button in the toolbar", 5);
        let best = result.best_match.expect("best match");
        assert_eq!(best.element_index, 2);
        assert_eq!(best.element_ref, "@e2");
    }
}