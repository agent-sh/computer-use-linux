use crate::remote_desktop::LogicalMonitor;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_registry;
use wayland_client::{event_created_child, Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::output_management::v1::client::{
    zwlr_output_head_v1, zwlr_output_manager_v1, zwlr_output_mode_v1,
};

const LAYOUT_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
pub(crate) struct RecordedHead {
    pub x: i32,
    pub y: i32,
    pub mode_width: i32,
    pub mode_height: i32,
    pub scale: f64,
    pub transform: u32,
    pub enabled: bool,
}

/// Logical size is the current mode divided by the head scale. Odd transforms
/// swap width and height. Disabled or unfinished heads are dropped.
pub(crate) fn logical_layout(heads: &[RecordedHead]) -> Option<Vec<LogicalMonitor>> {
    let mut layout = Vec::new();
    for head in heads {
        if !head.enabled || !head.scale.is_finite() || head.scale <= 0.0 {
            continue;
        }
        if head.mode_width <= 0 || head.mode_height <= 0 {
            continue;
        }
        let (mode_width, mode_height) = if head.transform % 2 == 1 {
            (head.mode_height, head.mode_width)
        } else {
            (head.mode_width, head.mode_height)
        };
        let width = (f64::from(mode_width) / head.scale).round() as i32;
        let height = (f64::from(mode_height) / head.scale).round() as i32;
        if width <= 0 || height <= 0 {
            continue;
        }
        layout.push(LogicalMonitor {
            x: head.x,
            y: head.y,
            width,
            height,
            scale: head.scale,
        });
    }
    layout.sort_by_key(|monitor| (monitor.x, monitor.y, monitor.width, monitor.height));
    (!layout.is_empty()).then_some(layout)
}

pub(crate) async fn wlr_output_layout() -> Option<Vec<LogicalMonitor>> {
    // blocking_dispatch can wait past the loop deadline; bound the whole task.
    tokio::time::timeout(LAYOUT_TIMEOUT, tokio::task::spawn_blocking(collect_layout))
        .await
        .ok()?
        .ok()
        .and_then(Result::ok)
}

fn collect_layout() -> Result<Vec<LogicalMonitor>> {
    let conn = Connection::connect_to_env().context("no Wayland display")?;
    let (globals, mut event_queue) =
        registry_queue_init(&conn).context("Wayland registry failed")?;
    let mut state = State::default();
    let qh = event_queue.handle();
    state.manager = globals
        .bind::<zwlr_output_manager_v1::ZwlrOutputManagerV1, _, _>(&qh, 1..=4, ())
        .ok();
    if state.manager.is_none() {
        anyhow::bail!("zwlr_output_manager_v1 is not advertised");
    }
    let deadline = Instant::now() + LAYOUT_TIMEOUT;
    while !state.done && Instant::now() < deadline {
        event_queue
            .blocking_dispatch(&mut state)
            .context("Wayland dispatch failed")?;
    }
    state.layout().context("wlr output layout was not ready")
}

#[derive(Default)]
struct State {
    manager: Option<zwlr_output_manager_v1::ZwlrOutputManagerV1>,
    done: bool,
    heads: HashMap<u32, Head>,
    modes: HashMap<u32, Mode>,
}

#[derive(Default)]
struct Head {
    enabled: bool,
    finished: bool,
    position: Option<(i32, i32)>,
    transform: Option<u32>,
    scale: Option<f64>,
    current_mode: Option<u32>,
}

#[derive(Default)]
struct Mode {
    finished: bool,
    size: Option<(i32, i32)>,
}

impl State {
    fn layout(&self) -> Option<Vec<LogicalMonitor>> {
        if !self.done {
            return None;
        }
        let heads = self
            .heads
            .values()
            .filter(|head| !head.finished)
            .filter_map(|head| {
                let (x, y) = head.position?;
                let (mode_width, mode_height) = self.modes.get(&head.current_mode?)?.size?;
                Some(RecordedHead {
                    x,
                    y,
                    mode_width,
                    mode_height,
                    scale: head.scale?,
                    transform: head.transform.unwrap_or(0),
                    enabled: head.enabled,
                })
            })
            .collect::<Vec<_>>();
        logical_layout(&heads)
    }
}

impl Dispatch<zwlr_output_manager_v1::ZwlrOutputManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &zwlr_output_manager_v1::ZwlrOutputManagerV1,
        event: zwlr_output_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_output_manager_v1::Event::Head { head } => {
                state.done = false;
                state.heads.entry(head.id().protocol_id()).or_default();
            }
            zwlr_output_manager_v1::Event::Done { .. } => state.done = true,
            zwlr_output_manager_v1::Event::Finished => state.done = false,
            _ => {}
        }
    }

    event_created_child!(
        State,
        zwlr_output_manager_v1::ZwlrOutputManagerV1,
        [zwlr_output_manager_v1::EVT_HEAD_OPCODE => (zwlr_output_head_v1::ZwlrOutputHeadV1, ())]
    );
}

impl Dispatch<zwlr_output_head_v1::ZwlrOutputHeadV1, ()> for State {
    fn event(
        state: &mut Self,
        head: &zwlr_output_head_v1::ZwlrOutputHeadV1,
        event: zwlr_output_head_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.done = false;
        let slot = state.heads.entry(head.id().protocol_id()).or_default();
        match event {
            zwlr_output_head_v1::Event::Mode { mode } => {
                state.modes.entry(mode.id().protocol_id()).or_default();
            }
            zwlr_output_head_v1::Event::Enabled { enabled } => slot.enabled = enabled != 0,
            zwlr_output_head_v1::Event::CurrentMode { mode } => {
                slot.current_mode = Some(mode.id().protocol_id());
            }
            zwlr_output_head_v1::Event::Position { x, y } => slot.position = Some((x, y)),
            zwlr_output_head_v1::Event::Transform { transform } => {
                slot.transform = Some(transform.into());
            }
            zwlr_output_head_v1::Event::Scale { scale } => slot.scale = Some(scale),
            zwlr_output_head_v1::Event::Finished => slot.finished = true,
            _ => {}
        }
    }

    event_created_child!(
        State,
        zwlr_output_head_v1::ZwlrOutputHeadV1,
        [zwlr_output_head_v1::EVT_MODE_OPCODE => (zwlr_output_mode_v1::ZwlrOutputModeV1, ())]
    );
}

impl Dispatch<zwlr_output_mode_v1::ZwlrOutputModeV1, ()> for State {
    fn event(
        state: &mut Self,
        mode: &zwlr_output_mode_v1::ZwlrOutputModeV1,
        event: zwlr_output_mode_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.done = false;
        let slot = state.modes.entry(mode.id().protocol_id()).or_default();
        match event {
            zwlr_output_mode_v1::Event::Size { width, height } => {
                slot.size = Some((width, height));
            }
            zwlr_output_mode_v1::Event::Finished => slot.finished = true,
            _ => {}
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_layout_divides_mode_by_scale() {
        let layout = logical_layout(&[
            RecordedHead {
                x: 0,
                y: 0,
                mode_width: 2048,
                mode_height: 1536,
                scale: 2.0,
                transform: 0,
                enabled: true,
            },
            RecordedHead {
                x: 10,
                y: 0,
                mode_width: 800,
                mode_height: 600,
                scale: 1.0,
                transform: 0,
                enabled: false,
            },
        ])
        .unwrap();
        assert_eq!(layout.len(), 1);
        assert_eq!(layout[0].width, 1024);
        assert_eq!(layout[0].height, 768);
        assert_eq!(layout[0].scale, 2.0);
    }
}
