//! A portal-owned EIS connection. Input starts only after the requested devices
//! have resumed, rather than treating the transport handshake as device readiness.
use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use reis::{
    ei,
    event::{Device, DeviceCapability, EiEvent},
};
use std::{os::unix::net::UnixStream, time::Duration};
use tokio::sync::{mpsc, oneshot};

#[derive(Clone, Copy)]
pub(crate) enum DeviceKind {
    Keyboard,
    Pointer,
}

#[derive(Clone, Copy)]
pub(crate) enum Input {
    Keysym(i32, u32),
    Keycode(i32, u32),
    Motion(f64, f64),
    Button(i32, u32),
    Scroll(u32, i32),
}

struct InputDevice {
    device: Device,
    active: bool,
    sequence: u32,
}

type Receipt = oneshot::Sender<std::result::Result<(), String>>;
#[derive(Clone)]
pub(crate) struct EiInput {
    sender: mpsc::Sender<(Input, Receipt)>,
}

impl EiInput {
    pub(crate) async fn connect(stream: UnixStream, kind: DeviceKind) -> Result<Self> {
        // reis's event stream is !Send. Give each granted session a private
        // thread; dropping every sender closes its channel and ends the thread.
        let (sender, receiver) = mpsc::channel(1);
        let (ready, started) = oneshot::channel();
        std::thread::Builder::new()
            .name("cul-eis-input".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = ready.send(Err(error.to_string()));
                        return;
                    }
                };
                runtime.block_on(worker(stream, kind, receiver, ready));
            })
            .context("could not start EIS input worker")?;
        tokio::time::timeout(Duration::from_secs(5), started)
            .await
            .context("EIS devices did not resume before the input startup deadline")?
            .context("EIS worker ended before device readiness")?
            .map_err(anyhow::Error::msg)?;
        Ok(Self { sender })
    }

    pub(crate) async fn send(&self, input: Input) -> Result<()> {
        let (receipt, received) = oneshot::channel();
        tokio::time::timeout(Duration::from_secs(3), async {
            self.sender
                .send((input, receipt))
                .await
                .context("EIS input connection closed")?;
            received
                .await
                .context("EIS input worker stopped")?
                .map_err(anyhow::Error::msg)
        })
        .await
        .context("EIS input dispatch timed out; re-observe before retrying")?
    }
}

async fn worker(
    stream: UnixStream,
    kind: DeviceKind,
    mut requests: mpsc::Receiver<(Input, Receipt)>,
    ready: oneshot::Sender<std::result::Result<(), String>>,
) {
    let startup = async {
        let context = ei::Context::new(stream)?;
        let (connection, mut events) = context
            .handshake_tokio("computer-use-linux", ei::handshake::ContextType::Sender)
            .await
            .map_err(|error| anyhow!("EIS handshake failed: {error}"))?;
        let mut devices = Vec::new();
        loop {
            let event = events
                .next()
                .await
                .context("EIS ended before devices resumed")??;
            handle_event(event, kind, &connection, &mut devices)?;
            if device_ready(kind, &devices) {
                break;
            }
        }
        Ok::<_, anyhow::Error>((connection, events, devices))
    };
    let (connection, mut events, mut devices) =
        match tokio::time::timeout(Duration::from_secs(5), startup).await {
            Ok(Ok(state)) => state,
            Ok(Err(error)) => {
                let _ = ready.send(Err(format!("{error:#}")));
                return;
            }
            Err(_) => {
                let _ = ready.send(Err("EIS device readiness timed out".into()));
                return;
            }
        };
    if ready.send(Ok(())).is_err() {
        return;
    }
    loop {
        tokio::select! {
            biased;
            event = events.next() => {
                let Some(Ok(event)) = event else { break };
                if handle_event(event, kind, &connection, &mut devices).is_err() { break; }
            }
            request = requests.recv() => {
                let Some((input, receipt)) = request else { break };
                // A caller that timed out must not leave queued input behind.
                if receipt.is_closed() { continue; }
                let result = emit(input, &connection, &devices).map_err(|error| format!("{error:#}"));
                let failed = result.is_err();
                let _ = receipt.send(result);
                if failed { break; }
            }
        }
    }
}

fn handle_event(
    event: EiEvent,
    kind: DeviceKind,
    connection: &reis::event::Connection,
    devices: &mut Vec<InputDevice>,
) -> Result<()> {
    match event {
        EiEvent::Disconnected(event) => bail!(
            "EIS input disconnected: {:?} {:?}",
            event.reason,
            event.explanation
        ),
        EiEvent::SeatRemoved(_) => bail!("EIS input seat was removed"),
        EiEvent::SeatAdded(event) => {
            let capabilities = match kind {
                DeviceKind::Keyboard => DeviceCapability::Keyboard | DeviceCapability::Text,
                DeviceKind::Pointer => {
                    DeviceCapability::PointerAbsolute
                        | DeviceCapability::Button
                        | DeviceCapability::Scroll
                }
            };
            event.seat.bind_capabilities(capabilities);
        }
        EiEvent::DeviceAdded(event) => {
            if event.device.device().version() >= 3 {
                event.device.device().ready();
            }
            devices.push(InputDevice {
                device: event.device,
                active: false,
                sequence: 0,
            });
        }
        EiEvent::DeviceResumed(event) => {
            if let Some(state) = devices
                .iter_mut()
                .find(|state| state.device == event.device)
            {
                event
                    .device
                    .device()
                    .start_emulating(event.serial, state.sequence);
                state.sequence = state.sequence.wrapping_add(1);
                state.active = true;
            }
        }
        EiEvent::DevicePaused(event) => {
            if let Some(state) = devices
                .iter_mut()
                .find(|state| state.device == event.device)
            {
                state.active = false;
            }
        }
        EiEvent::DeviceRemoved(event) => {
            devices.retain(|state| state.device != event.device);
        }
        _ => {}
    }
    connection
        .flush()
        .context("could not flush EIS device negotiation")?;
    Ok(())
}

fn device_ready(kind: DeviceKind, devices: &[InputDevice]) -> bool {
    let available = |name: &str| {
        devices.iter().any(|state| {
            let device = &state.device;
            state.active
                && match name {
                    "text" => device.interface::<ei::Text>().is_some(),
                    "keyboard" => device.interface::<ei::Keyboard>().is_some(),
                    "pointer" => device.interface::<ei::PointerAbsolute>().is_some(),
                    "button" => device.interface::<ei::Button>().is_some(),
                    "scroll" => device.interface::<ei::Scroll>().is_some(),
                    _ => false,
                }
        })
    };
    match kind {
        DeviceKind::Keyboard => available("text") && available("keyboard"),
        DeviceKind::Pointer => available("pointer") && available("button") && available("scroll"),
    }
}

fn emit(input: Input, connection: &reis::event::Connection, devices: &[InputDevice]) -> Result<()> {
    let key_state = |state| {
        if state == 0 {
            ei::keyboard::KeyState::Released
        } else {
            ei::keyboard::KeyState::Press
        }
    };
    let mut emitted = false;
    for state in devices {
        let device = &state.device;
        if !state.active {
            continue;
        }
        if let Input::Motion(x, y) = input {
            if !device.regions().iter().any(|region| {
                x >= region.x as f64
                    && y >= region.y as f64
                    && x < region.x as f64 + region.width as f64
                    && y < region.y as f64 + region.height as f64
            }) {
                continue;
            }
        }
        match input {
            Input::Keysym(key, state) => {
                if let Some(interface) = device.interface::<ei::Text>() {
                    interface.keysym(key as u32, key_state(state));
                }
            }
            Input::Keycode(key, state) => {
                if let Some(interface) = device.interface::<ei::Keyboard>() {
                    interface.key(key as u32, key_state(state));
                }
            }
            Input::Motion(x, y) => {
                if let Some(interface) = device.interface::<ei::PointerAbsolute>() {
                    interface.motion_absolute(x as f32, y as f32);
                }
            }
            Input::Button(button, state) => {
                if let Some(interface) = device.interface::<ei::Button>() {
                    interface.button(
                        button as u32,
                        if state == 0 {
                            ei::button::ButtonState::Released
                        } else {
                            ei::button::ButtonState::Press
                        },
                    );
                }
            }
            Input::Scroll(axis, steps) => {
                if let Some(interface) = device.interface::<ei::Scroll>() {
                    let value = steps.saturating_mul(120);
                    interface.scroll_discrete(
                        if axis == 1 { value } else { 0 },
                        if axis == 0 { value } else { 0 },
                    );
                }
            }
        }
        // A device can carry only some requested interfaces. Do not frame a
        // different device or claim success when that interface was paused.
        let has_interface = match input {
            Input::Keysym(..) => device.interface::<ei::Text>().is_some(),
            Input::Keycode(..) => device.interface::<ei::Keyboard>().is_some(),
            Input::Motion(..) => device.interface::<ei::PointerAbsolute>().is_some(),
            Input::Button(..) => device.interface::<ei::Button>().is_some(),
            Input::Scroll(..) => device.interface::<ei::Scroll>().is_some(),
        };
        if has_interface {
            let mut now = libc::timespec {
                tv_sec: 0,
                tv_nsec: 0,
            };
            // SAFETY: now points to an initialized, writable timespec.
            if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now) } != 0 {
                bail!("could not timestamp EIS input");
            }
            let micros = (now.tv_sec as u64).saturating_mul(1_000_000) + now.tv_nsec as u64 / 1000;
            device.device().frame(connection.serial(), micros);
            emitted = true;
            break;
        }
    }
    if !emitted {
        bail!("EIS input device is not resumed; no input was sent");
    }
    connection.flush().context("could not flush EIS input")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use reis::{
        eis,
        handshake::EisHandshaker,
        request::{EisRequest, EisRequestConverter},
        PendingRequestResult,
    };
    use std::{os::fd::AsRawFd, time::Instant};

    fn next(context: &eis::Context) -> eis::Request {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(request) = context.pending_request() {
                return match request {
                    PendingRequestResult::Request(request) => request,
                    other => panic!("invalid EIS request: {other:?}"),
                };
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "EIS peer timed out");
            let mut fd = libc::pollfd {
                fd: context.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: fd points to a valid, writable pollfd for this socket.
            let ready = unsafe { libc::poll(&mut fd, 1, remaining.as_millis() as i32) };
            assert!(ready > 0, "EIS peer did not receive a request");
            context.read().unwrap();
        }
    }

    #[tokio::test]
    async fn first_key_waits_for_v3_ready_and_device_resumption() {
        let (client, server) = UnixStream::pair().unwrap();
        let (advertised, ready_seen) = oneshot::channel();
        let (resume, permitted) = std::sync::mpsc::channel();
        let (recorded, received) = oneshot::channel();
        let (finish, finished) = std::sync::mpsc::channel();
        let peer = std::thread::spawn(move || {
            let context = eis::Context::new(server).unwrap();
            let mut handshake = EisHandshaker::new(&context, 1);
            let response = loop {
                if let Some(response) = handshake.handle_request(next(&context)).unwrap() {
                    break response;
                }
                context.flush().unwrap();
            };
            let mut converter = EisRequestConverter::new(&context, response, 1);
            let connection = converter.handle().clone();
            let _seat = connection.add_seat(
                Some("fixture"),
                DeviceCapability::Keyboard | DeviceCapability::Text,
            );
            connection.flush().unwrap();
            let mut advertised = Some(advertised);
            let mut keys = Vec::new();
            loop {
                converter.handle_request(next(&context)).unwrap();
                while let Some(request) = converter.next_request() {
                    match request {
                        EisRequest::Bind(request) => {
                            // A v3 peer must see ready before it can resume the device.
                            let device = request.seat.add_device(
                                Some("keyboard"),
                                eis::device::DeviceType::Virtual,
                                DeviceCapability::Keyboard | DeviceCapability::Text,
                                |_| {},
                            );
                            assert_eq!(device.device().version(), 3);
                            connection.flush().unwrap();
                        }
                        EisRequest::Ready(request) => {
                            advertised.take().unwrap().send(()).unwrap();
                            permitted.recv_timeout(Duration::from_secs(2)).unwrap();
                            request.device.resumed();
                            connection.flush().unwrap();
                        }
                        EisRequest::TextKeysym(request) => {
                            if request.state == eis::keyboard::KeyState::Press {
                                keys.push(request.keysym);
                            }
                            if keys.len() == 6 {
                                connection.connection().disconnected(
                                    connection.last_serial(),
                                    eis::connection::DisconnectReason::Disconnected,
                                    None,
                                );
                                connection.flush().unwrap();
                                recorded.send(keys).unwrap();
                                // Keep the socket alive. A terminal protocol event,
                                // rather than EOF, must stop the input worker.
                                finished.recv_timeout(Duration::from_secs(3)).unwrap();
                                return;
                            }
                        }
                        _ => {}
                    }
                }
                connection.flush().unwrap();
            }
        });
        let connecting = EiInput::connect(client, DeviceKind::Keyboard);
        tokio::pin!(connecting);
        tokio::select! {
            _ = &mut connecting => panic!("transport handshake was mistaken for device readiness"),
            result = ready_seen => result.unwrap(),
        }
        resume.send(()).unwrap();
        let input = connecting.await.unwrap();
        for key in b"CUL312" {
            input.send(Input::Keysym(i32::from(*key), 1)).await.unwrap();
            // The peer may close after the last press; every preceding key must
            // still have its matching release before the next one is sent.
            if *key != b'2' {
                input.send(Input::Keysym(i32::from(*key), 0)).await.unwrap();
            }
        }
        assert_eq!(
            received.await.unwrap(),
            b"CUL312"
                .iter()
                .map(|value| u32::from(*value))
                .collect::<Vec<_>>()
        );
        tokio::time::timeout(Duration::from_secs(3), input.sender.closed())
            .await
            .unwrap();
        assert!(input.send(Input::Keysym(i32::from(b'x'), 1)).await.is_err());
        finish.send(()).unwrap();
        peer.join().unwrap();
    }
}
