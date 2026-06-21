use crate::diagnostics::hydrate_session_bus_env;
use anyhow::{anyhow, Context, Result};
use atspi::{
    proxy::{
        accessible::{AccessibleProxy, ObjectRefExt},
        proxy_ext::ProxyExt,
    },
    AccessibilityConnection, CoordType, ObjectRef, ObjectRefOwned, StateSet,
};
use schemars::JsonSchema;
use serde::Serialize;
use std::collections::VecDeque;
use zbus::{
    fdo::DBusProxy,
    names::{BusName, UniqueName},
    zvariant::ObjectPath,
};

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct AccessibleAppSummary {
    pub object_ref: String,
    pub name: Option<String>,
    pub pid: Option<u32>,
    pub role: String,
    pub child_count: i32,
    pub bounds: Option<Bounds>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct AccessibilityNode {
    pub index: u32,
    pub parent_index: Option<u32>,
    pub depth: u32,
    pub object_ref: String,
    pub role: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub child_count: i32,
    pub bounds: Option<Bounds>,
    pub states: Vec<String>,
    pub actions: Vec<AccessibilityAction>,
    pub value: Option<AccessibilityValue>,
    pub text: Option<AccessibilityText>,
    pub supports_editable_text: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct AccessibilityAction {
    pub index: i32,
    pub name: String,
    pub description: String,
    pub keybinding: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct AccessibilityValue {
    pub current: f64,
    pub minimum: f64,
    pub maximum: f64,
    pub minimum_increment: f64,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct AccessibilityText {
    pub character_count: i32,
    pub caret_offset: Option<i32>,
    pub content: Option<String>,
    pub truncated: bool,
    pub selections: Vec<AccessibilityTextSelection>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct AccessibilityTextSelection {
    pub start_offset: i32,
    pub end_offset: i32,
}

#[derive(Debug, Clone)]
pub struct ActionInvocation {
    pub action_index: i32,
    pub action_name: Option<String>,
    pub ok: bool,
}

#[derive(Debug, Clone)]
pub enum ValueSetInvocation {
    Numeric { value: f64 },
    EditableText,
}

const MAX_TEXT_READBACK_CHARS: i32 = 4096;
const MAX_TEXT_SELECTIONS: i32 = 8;

pub async fn list_accessible_apps(limit: usize) -> Result<Vec<AccessibleAppSummary>> {
    let conn = connect().await?;
    let roots = registry_children(&conn).await?;
    let dbus = DBusProxy::new(conn.connection()).await.ok();
    let mut apps = Vec::new();

    for object_ref in roots.into_iter().take(limit) {
        if let Ok(proxy) = open_accessible(&conn, &object_ref).await {
            apps.push(read_app_summary(&proxy, &object_ref, dbus.as_ref()).await);
        }
    }

    Ok(apps)
}

pub async fn snapshot_tree(
    app_name_or_bundle_identifier: Option<&str>,
    target_pid: Option<u32>,
    max_nodes: usize,
    max_depth: u32,
) -> Result<Vec<AccessibilityNode>> {
    let conn = connect().await?;
    let roots = registry_children(&conn).await?;
    let selected_roots =
        select_roots(&conn, roots, app_name_or_bundle_identifier, target_pid).await;
    let mut nodes = Vec::new();
    let mut queue = VecDeque::new();

    for object_ref in selected_roots {
        queue.push_back((object_ref, 0_u32, None));
    }

    while let Some((object_ref, depth, parent_index)) = queue.pop_front() {
        if nodes.len() >= max_nodes {
            break;
        }

        let Ok(proxy) = open_accessible(&conn, &object_ref).await else {
            continue;
        };
        let index = nodes.len() as u32;
        let child_refs = if depth < max_depth {
            proxy.get_children().await.unwrap_or_default()
        } else {
            Vec::new()
        };

        nodes.push(read_node(&proxy, &object_ref, index, parent_index, depth).await);

        for child in child_refs {
            queue.push_back((child, depth + 1, Some(index)));
        }
    }

    Ok(nodes)
}

pub async fn perform_action(
    object_ref_id: &str,
    requested_action: Option<&str>,
) -> Result<ActionInvocation> {
    let conn = connect().await?;
    let object_ref = object_ref_from_id(object_ref_id)?;
    let proxy = open_accessible(&conn, &object_ref)
        .await
        .with_context(|| format!("failed to open AT-SPI object {object_ref_id}"))?;
    let action = proxy
        .proxies()
        .await?
        .action()
        .await
        .context("element does not expose the AT-SPI Action interface")?;
    let actions = action.get_actions().await.unwrap_or_default();
    let action_index = select_action_index(&actions, requested_action)?;
    let action_name = actions
        .get(action_index as usize)
        .map(|action| action.name.clone());
    let ok = action
        .do_action(action_index)
        .await
        .with_context(|| format!("failed to invoke AT-SPI action {action_index}"))?;

    Ok(ActionInvocation {
        action_index,
        action_name,
        ok,
    })
}

pub async fn set_element_value(object_ref_id: &str, value: &str) -> Result<ValueSetInvocation> {
    let conn = connect().await?;
    let object_ref = object_ref_from_id(object_ref_id)?;
    let proxy = open_accessible(&conn, &object_ref)
        .await
        .with_context(|| format!("failed to open AT-SPI object {object_ref_id}"))?;
    let proxies = proxy.proxies().await?;

    if let Ok(numeric_value) = value.parse::<f64>() {
        if let Ok(value_proxy) = proxies.value().await {
            value_proxy
                .set_current_value(numeric_value)
                .await
                .with_context(|| {
                    format!("failed to set AT-SPI numeric value to {numeric_value}")
                })?;
            return Ok(ValueSetInvocation::Numeric {
                value: numeric_value,
            });
        }
    }

    if let Ok(editable_text) = proxies.editable_text().await {
        let ok = editable_text
            .set_text_contents(value)
            .await
            .context("failed to set AT-SPI editable text contents")?;
        if ok {
            return Ok(ValueSetInvocation::EditableText);
        }
        return Err(anyhow!("AT-SPI EditableText rejected the new contents"));
    }

    if value.parse::<f64>().is_err() && proxies.value().await.is_ok() {
        return Err(anyhow!(
            "element exposes the AT-SPI Value interface, but the requested value is not numeric"
        ));
    }

    Err(anyhow!(
        "element does not expose AT-SPI Value or EditableText interfaces"
    ))
}

async fn connect() -> Result<AccessibilityConnection> {
    hydrate_session_bus_env();
    AccessibilityConnection::new()
        .await
        .context("failed to connect to AT-SPI bus")
}

/// Open an `AccessibleProxy` for an object on the a11y bus.
///
/// We deliberately avoid `AccessibilityConnection::object_as_accessible` (the
/// `P2P` trait). For apps that advertise a peer-to-peer bus address it routes
/// reads over that socket, but for apps that don't (notably GTK4 apps such as
/// Nautilus / Text Editor / baobab, which don't implement the legacy
/// `GetApplicationBusAddress`) it falls back to a proxy built with only a path
/// and *no destination*. On the shared a11y bus that proxy can't address the
/// app and every call fails with `ServiceUnknown`, which surfaces as an empty
/// tree (`role: "unknown"`, `child_count: 0`). `as_accessible_proxy` always
/// pins the destination to the object's bus name, so it works for every app
/// regardless of P2P support. See issue #31.
async fn open_accessible<'r>(
    conn: &AccessibilityConnection,
    object_ref: &'r ObjectRefOwned,
) -> Result<AccessibleProxy<'r>, atspi::AtspiError> {
    object_ref.as_accessible_proxy(conn.connection()).await
}

async fn registry_children(conn: &AccessibilityConnection) -> Result<Vec<ObjectRefOwned>> {
    let root = conn
        .root_accessible_on_registry()
        .await
        .context("failed to open AT-SPI registry root")?;
    root.get_children()
        .await
        .context("failed to read AT-SPI registry children")
}

async fn select_roots(
    conn: &AccessibilityConnection,
    roots: Vec<ObjectRefOwned>,
    app_name_or_bundle_identifier: Option<&str>,
    target_pid: Option<u32>,
) -> Vec<ObjectRefOwned> {
    let needle = app_name_or_bundle_identifier
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase());
    let dbus = DBusProxy::new(conn.connection()).await.ok();
    let mut remaining = roots;

    if let Some(target_pid) = target_pid {
        let mut pid_and_filter_matches = Vec::new();
        let mut pid_matches = Vec::new();
        let mut non_pid_matches = Vec::new();

        for object_ref in remaining {
            if object_ref_pid(dbus.as_ref(), &object_ref).await == Some(target_pid) {
                if let Some(needle) = needle.as_deref() {
                    if root_matches(conn, &object_ref, needle).await {
                        pid_and_filter_matches.push(object_ref);
                    } else {
                        pid_matches.push(object_ref);
                    }
                } else {
                    pid_matches.push(object_ref);
                }
            } else {
                non_pid_matches.push(object_ref);
            }
        }

        if !pid_and_filter_matches.is_empty() {
            return pid_and_filter_matches;
        }
        if !pid_matches.is_empty() {
            return pid_matches;
        }

        remaining = non_pid_matches;
    }

    let Some(needle) = needle.as_deref() else {
        return remaining;
    };

    let mut selected = Vec::new();
    for object_ref in remaining {
        if root_matches(conn, &object_ref, needle).await {
            selected.push(object_ref);
        }
    }

    selected
}

async fn root_matches(
    conn: &AccessibilityConnection,
    object_ref: &ObjectRefOwned,
    needle: &str,
) -> bool {
    let Ok(proxy) = open_accessible(conn, object_ref).await else {
        return object_ref_id(object_ref)
            .to_ascii_lowercase()
            .contains(needle);
    };

    if proxy_matches(&proxy, object_ref, needle).await {
        return true;
    }

    let children = proxy.get_children().await.unwrap_or_default();
    for child_ref in children.into_iter().take(8) {
        let Ok(child_proxy) = open_accessible(conn, &child_ref).await else {
            continue;
        };
        if proxy_matches(&child_proxy, &child_ref, needle).await {
            return true;
        }
    }

    false
}

async fn proxy_matches(
    proxy: &AccessibleProxy<'_>,
    object_ref: &ObjectRefOwned,
    needle: &str,
) -> bool {
    let name = proxy.name().await.unwrap_or_default();
    let role = proxy.get_role_name().await.unwrap_or_default();
    format!("{} {} {}", object_ref_id(object_ref), name, role)
        .to_ascii_lowercase()
        .contains(needle)
}

async fn read_app_summary(
    proxy: &AccessibleProxy<'_>,
    object_ref: &ObjectRefOwned,
    dbus: Option<&DBusProxy<'_>>,
) -> AccessibleAppSummary {
    AccessibleAppSummary {
        object_ref: object_ref_id(object_ref),
        name: optional_string(proxy.name().await.ok()),
        pid: object_ref_pid(dbus, object_ref).await,
        role: role_name(proxy).await,
        child_count: proxy.child_count().await.unwrap_or_default(),
        bounds: bounds(proxy).await,
    }
}

async fn read_node(
    proxy: &AccessibleProxy<'_>,
    object_ref: &ObjectRefOwned,
    index: u32,
    parent_index: Option<u32>,
    depth: u32,
) -> AccessibilityNode {
    let proxies = proxy.proxies().await.ok();

    AccessibilityNode {
        index,
        parent_index,
        depth,
        object_ref: object_ref_id(object_ref),
        role: role_name(proxy).await,
        name: optional_string(proxy.name().await.ok()),
        description: optional_string(proxy.description().await.ok()),
        child_count: proxy.child_count().await.unwrap_or_default(),
        bounds: bounds_from_proxies(proxies.as_ref(), proxy).await,
        states: states_from_proxy(proxy).await,
        actions: actions_from_proxies(proxies.as_ref()).await,
        value: value_from_proxies(proxies.as_ref()).await,
        text: text_from_proxies(proxies.as_ref()).await,
        supports_editable_text: supports_editable_text(proxies.as_ref()).await,
    }
}

async fn role_name(proxy: &AccessibleProxy<'_>) -> String {
    if let Ok(role) = proxy.get_role_name().await {
        if !role.trim().is_empty() {
            return role;
        }
    }
    proxy
        .get_role()
        .await
        .map(|role| format!("{role:?}"))
        .unwrap_or_else(|_| "unknown".to_string())
}

async fn bounds(proxy: &AccessibleProxy<'_>) -> Option<Bounds> {
    bounds_from_proxies(proxy.proxies().await.ok().as_ref(), proxy).await
}

async fn object_ref_pid(dbus: Option<&DBusProxy<'_>>, object_ref: &ObjectRefOwned) -> Option<u32> {
    let dbus = dbus?;
    let bus_name = BusName::try_from(object_ref.name_as_str()?.to_string()).ok()?;
    dbus.get_connection_unix_process_id(bus_name).await.ok()
}

async fn bounds_from_proxies(
    proxies: Option<&atspi::proxy::proxy_ext::Proxies<'_>>,
    proxy: &AccessibleProxy<'_>,
) -> Option<Bounds> {
    let owned_proxies;
    let proxies = if let Some(proxies) = proxies {
        proxies
    } else {
        owned_proxies = proxy.proxies().await.ok()?;
        &owned_proxies
    };
    let component = proxies.component().await.ok()?;
    let (x, y, width, height) = component.get_extents(CoordType::Screen).await.ok()?;
    normalize_bounds(Bounds {
        x,
        y,
        width,
        height,
    })
}

fn normalize_bounds(bounds: Bounds) -> Option<Bounds> {
    if bounds.width <= 0 || bounds.height <= 0 {
        return None;
    }
    if bounds.x <= i32::MIN / 2 || bounds.y <= i32::MIN / 2 {
        return None;
    }
    Some(bounds)
}

async fn actions_from_proxies(
    proxies: Option<&atspi::proxy::proxy_ext::Proxies<'_>>,
) -> Vec<AccessibilityAction> {
    let Some(proxies) = proxies else {
        return Vec::new();
    };
    let Ok(action_proxy) = proxies.action().await else {
        return Vec::new();
    };

    action_proxy
        .get_actions()
        .await
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .map(|(index, action)| AccessibilityAction {
            index: index as i32,
            name: action.name,
            description: action.description,
            keybinding: action.keybinding,
        })
        .collect()
}

async fn states_from_proxy(proxy: &AccessibleProxy<'_>) -> Vec<String> {
    proxy
        .get_state()
        .await
        .map(state_labels)
        .unwrap_or_default()
}

async fn value_from_proxies(
    proxies: Option<&atspi::proxy::proxy_ext::Proxies<'_>>,
) -> Option<AccessibilityValue> {
    let value = proxies?.value().await.ok()?;
    Some(AccessibilityValue {
        current: value.current_value().await.ok()?,
        minimum: value.minimum_value().await.ok()?,
        maximum: value.maximum_value().await.ok()?,
        minimum_increment: value.minimum_increment().await.ok()?,
        text: optional_string(value.text().await.ok()),
    })
}

async fn text_from_proxies(
    proxies: Option<&atspi::proxy::proxy_ext::Proxies<'_>>,
) -> Option<AccessibilityText> {
    let text = proxies?.text().await.ok()?;
    let character_count = text.character_count().await.ok()?.max(0);
    let caret_offset = text.caret_offset().await.ok();
    let capped_count = character_count.min(MAX_TEXT_READBACK_CHARS);
    let content = if capped_count > 0 {
        optional_string(text.get_text(0, capped_count).await.ok())
    } else {
        None
    };
    let selection_count = text
        .get_nselections()
        .await
        .unwrap_or_default()
        .clamp(0, MAX_TEXT_SELECTIONS);
    let mut selections = Vec::new();
    for index in 0..selection_count {
        if let Ok((start_offset, end_offset)) = text.get_selection(index).await {
            selections.push(AccessibilityTextSelection {
                start_offset,
                end_offset,
            });
        }
    }

    Some(AccessibilityText {
        character_count,
        caret_offset,
        content,
        truncated: character_count > MAX_TEXT_READBACK_CHARS,
        selections,
    })
}

async fn supports_editable_text(proxies: Option<&atspi::proxy::proxy_ext::Proxies<'_>>) -> bool {
    let Some(proxies) = proxies else {
        return false;
    };
    proxies.editable_text().await.is_ok()
}

fn state_labels(state_set: StateSet) -> Vec<String> {
    state_set.iter().map(|state| state.to_string()).collect()
}

fn select_action_index(actions: &[atspi::Action], requested_action: Option<&str>) -> Result<i32> {
    if actions.is_empty() {
        return Err(anyhow!("element exposes no AT-SPI actions"));
    }

    if let Some(requested_action) = requested_action
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let requested_action = requested_action.to_ascii_lowercase();
        if let Some((index, _)) = actions.iter().enumerate().find(|(_, action)| {
            action.name.to_ascii_lowercase() == requested_action
                || action.description.to_ascii_lowercase() == requested_action
        }) {
            return Ok(index as i32);
        }

        if let Ok(index) = requested_action.parse::<usize>() {
            if index < actions.len() {
                return Ok(index as i32);
            }
        }

        return Err(anyhow!(
            "requested AT-SPI action was not found; available actions: {}",
            actions
                .iter()
                .map(|action| action.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    Ok(if actions.len() > 1 { 1 } else { 0 })
}

fn optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn object_ref_from_id(object_ref_id: &str) -> Result<ObjectRefOwned> {
    let (name, path) = split_object_ref_id(object_ref_id)?;
    let name = UniqueName::try_from(name.to_string())
        .with_context(|| format!("invalid AT-SPI bus name in object ref {object_ref_id}"))?;
    let path = ObjectPath::try_from(path.to_string())
        .with_context(|| format!("invalid AT-SPI object path in object ref {object_ref_id}"))?;
    Ok(ObjectRef::new_owned(name, path))
}

fn split_object_ref_id(object_ref_id: &str) -> Result<(&str, &str)> {
    let Some(path_start) = object_ref_id.find('/') else {
        return Err(anyhow!(
            "invalid AT-SPI object ref '{object_ref_id}'; expected ':bus/path'"
        ));
    };
    let (name, path) = object_ref_id.split_at(path_start);
    if name.is_empty() || path.is_empty() {
        return Err(anyhow!(
            "invalid AT-SPI object ref '{object_ref_id}'; expected ':bus/path'"
        ));
    }
    Ok((name, path))
}

fn object_ref_id(object_ref: &ObjectRefOwned) -> String {
    format!(
        "{}{}",
        object_ref.name_as_str().unwrap_or(""),
        object_ref.path_as_str()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_object_ref_id_separates_bus_name_and_path() {
        let (name, path) = split_object_ref_id(":1.42/org/a11y/atspi/accessible/7").unwrap();

        assert_eq!(name, ":1.42");
        assert_eq!(path, "/org/a11y/atspi/accessible/7");
    }

    #[test]
    fn select_action_index_uses_named_action() {
        let actions = vec![
            atspi::Action {
                name: "click".to_string(),
                description: "Clicks".to_string(),
                keybinding: String::new(),
            },
            atspi::Action {
                name: "show-menu".to_string(),
                description: "Shows menu".to_string(),
                keybinding: String::new(),
            },
        ];

        assert_eq!(select_action_index(&actions, Some("show-menu")).unwrap(), 1);
    }

    #[test]
    fn select_action_index_defaults_to_secondary_when_available() {
        let actions = vec![
            atspi::Action {
                name: "click".to_string(),
                description: String::new(),
                keybinding: String::new(),
            },
            atspi::Action {
                name: "show-menu".to_string(),
                description: String::new(),
                keybinding: String::new(),
            },
        ];

        assert_eq!(select_action_index(&actions, None).unwrap(), 1);
    }

    #[test]
    fn state_labels_serialize_in_bit_order() {
        let labels = state_labels(StateSet::new(atspi::State::Focused | atspi::State::Checked));

        assert_eq!(labels, vec!["checked".to_string(), "focused".to_string()]);
    }
}
