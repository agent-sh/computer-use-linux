use super::*;
use std::collections::HashSet;
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
};

const PORTAL_CLIPBOARD_INTERFACE: &str = "org.freedesktop.portal.Clipboard";
const CLIPBOARD_CALL_TIMEOUT: Duration = Duration::from_secs(5);
const INITIAL_OWNER_SETTLE_TIMEOUT: Duration = Duration::from_millis(250);
const CLEAR_OWNER_SETTLE_TIMEOUT: Duration = Duration::from_millis(250);
const SELECTION_READ_RETRY_DELAYS_MS: &[u64] = &[10, 15, 25, 40, 60, 80, 100, 150];
const MAX_CLIPBOARD_MIME_BYTES: usize = 16 * 1024 * 1024;
const MAX_CLIPBOARD_TOTAL_BYTES: usize = 64 * 1024 * 1024;
/// Uncached sentinel MIME type set immediately before unsetting an empty
/// clipboard so Mutter's `MetaClipboardManager` drops its saved copy of the
/// temporary pasted text instead of restoring it on `meta_selection_unset_owner`.
const EMPTY_CLIPBOARD_CLEAR_MIME: &str = "application/x-computer-use-linux-clear";
const ICCCM_META_TARGETS: &[&str] = &["TARGETS", "TIMESTAMP", "MULTIPLE", "SAVE_TARGETS", "DELETE"];

fn set_blocking(fd: &std::os::fd::OwnedFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    let result = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags & !libc::O_NONBLOCK) };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClipboardSelection {
    formats: Vec<(String, Vec<u8>)>,
}

impl ClipboardSelection {
    pub fn text(text: &str) -> Self {
        let utf8 = text.as_bytes().to_vec();
        Self {
            formats: vec![
                ("text/plain;charset=utf-8".to_string(), utf8.clone()),
                ("text/plain".to_string(), utf8.clone()),
                ("UTF8_STRING".to_string(), utf8),
            ],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.formats.is_empty()
    }

    fn mime_types(&self) -> Vec<String> {
        self.formats
            .iter()
            .map(|(mime_type, _)| mime_type.clone())
            .collect()
    }

    fn payload(&self, mime_type: &str) -> Option<&[u8]> {
        self.formats
            .iter()
            .find(|(candidate, _)| candidate == mime_type)
            .map(|(_, payload)| payload.as_slice())
    }
}

#[derive(Debug, Clone, Default)]
struct ClipboardOwner {
    mime_types: Vec<String>,
    session_is_owner: bool,
}

fn owned_string_list(value: &OwnedValue) -> Result<Vec<String>> {
    if let Ok(strings) = value.try_clone().and_then(Vec::<String>::try_from) {
        return Ok(strings);
    }
    let (strings,): (Vec<String>,) = value
        .try_clone()
        .and_then(<(Vec<String>,)>::try_from)
        .context("clipboard MIME list was neither an array nor a one-element array tuple")?;
    Ok(strings)
}

fn parse_owner_options(options: &HashMap<String, OwnedValue>) -> ClipboardOwner {
    let mime_types = options
        .get("mime_types")
        .or_else(|| options.get("mime-types"))
        .and_then(|value| owned_string_list(value).ok())
        .unwrap_or_default();
    let session_is_owner = options
        .get("session_is_owner")
        .or_else(|| options.get("session-is-owner"))
        .and_then(|value| bool::try_from(value).ok())
        .unwrap_or_default();
    ClipboardOwner {
        mime_types,
        session_is_owner,
    }
}

fn filter_readable_mime_types(mime_types: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut filtered = Vec::new();
    for mime_type in mime_types {
        if mime_type.is_empty() || ICCCM_META_TARGETS.contains(&mime_type.as_str()) {
            continue;
        }
        if seen.insert(mime_type.clone()) {
            filtered.push(mime_type.clone());
        }
    }
    filtered
}

fn is_transient_selection_read_error(error: &zbus::Error) -> bool {
    let text = error.to_string();
    text.contains("Tried to read in parallel")
        || text.contains("Internal error")
        || text.contains("Failed to selection read")
}

pub struct PortalClipboardSession {
    keyboard: PortalKeyboardSession,
    connection: Connection,
    session_handle: OwnedObjectPath,
    owner_changed: mpsc::UnboundedReceiver<ClipboardOwner>,
    owner_task: JoinHandle<()>,
    transfer_task: JoinHandle<()>,
    selection: watch::Sender<Arc<ClipboardSelection>>,
    latest_owner: Option<ClipboardOwner>,
    initial_owner_checked: bool,
    owner: bool,
}

impl PortalClipboardSession {
    pub fn keyboard(&self) -> &PortalKeyboardSession {
        &self.keyboard
    }

    pub fn is_valid(&self) -> bool {
        self.keyboard.is_valid()
            && !self.owner_task.is_finished()
            && !self.transfer_task.is_finished()
    }

    fn drain_owner_updates(&mut self) {
        while let Ok(owner) = self.owner_changed.try_recv() {
            self.owner = owner.session_is_owner;
            self.latest_owner = Some(owner);
        }
    }

    async fn wait_for_initial_owner(&mut self) {
        self.drain_owner_updates();
        if self.initial_owner_checked {
            return;
        }
        self.initial_owner_checked = true;
        if self.latest_owner.is_some() {
            return;
        }
        // Barrier on the portal connection so signals emitted during Start have
        // reached our socket before deciding the clipboard is empty.
        let _ = tokio::time::timeout(
            CLIPBOARD_CALL_TIMEOUT,
            self.connection.call_method(
                Some(PORTAL_DESKTOP_SERVICE),
                PORTAL_DESKTOP_PATH,
                Some("org.freedesktop.DBus.Peer"),
                "Ping",
                &(),
            ),
        )
        .await;
        self.drain_owner_updates();
        if self.latest_owner.is_some() {
            return;
        }
        if let Ok(Some(owner)) =
            tokio::time::timeout(INITIAL_OWNER_SETTLE_TIMEOUT, self.owner_changed.recv()).await
        {
            self.owner = owner.session_is_owner;
            self.latest_owner = Some(owner);
            self.drain_owner_updates();
        }
    }

    async fn read_mime_payload(&self, mime_type: &str, allow_retry: bool) -> Result<Vec<u8>> {
        let proxy = clipboard_proxy(&self.connection).await?;
        let mut attempt = 0usize;
        let fd: zbus::zvariant::OwnedFd = loop {
            let call = tokio::time::timeout(
                CLIPBOARD_CALL_TIMEOUT,
                proxy.call("SelectionRead", &(&self.session_handle, mime_type)),
            )
            .await
            .context("Clipboard SelectionRead call timed out")?;
            match call {
                Ok(fd) => break fd,
                Err(error)
                    if (allow_retry || is_transient_selection_read_error(&error))
                        && attempt < SELECTION_READ_RETRY_DELAYS_MS.len()
                        && is_transient_selection_read_error(&error) =>
                {
                    let delay = Duration::from_millis(SELECTION_READ_RETRY_DELAYS_MS[attempt]);
                    attempt += 1;
                    tokio::time::sleep(delay).await;
                }
                Err(error) => {
                    return Err(error).context("Clipboard SelectionRead call failed");
                }
            }
        };
        let fd: std::os::fd::OwnedFd = fd.into();
        set_blocking(&fd)?;
        let file = std::fs::File::from(fd);
        let payload = tokio::time::timeout(
            CLIPBOARD_CALL_TIMEOUT,
            tokio::task::spawn_blocking(move || {
                let mut payload = Vec::new();
                let mut limited = file.take(MAX_CLIPBOARD_MIME_BYTES as u64 + 1);
                limited.read_to_end(&mut payload)?;
                drop(limited);
                io::Result::Ok(payload)
            }),
        )
        .await
        .context("Clipboard payload read timed out")?
        .context("Clipboard payload reader task failed")??;
        if payload.len() > MAX_CLIPBOARD_MIME_BYTES {
            bail!("Clipboard MIME payload exceeds the 16 MiB restore limit");
        }
        Ok(payload)
    }

    pub async fn read_current_selection(&mut self) -> Result<ClipboardSelection> {
        self.wait_for_initial_owner().await;
        if self.owner {
            return Ok((**self.selection.borrow()).clone());
        }
        let mime_types = self
            .latest_owner
            .as_ref()
            .map(|owner| filter_readable_mime_types(&owner.mime_types))
            .unwrap_or_default();
        if mime_types.is_empty() {
            return Ok(ClipboardSelection::default());
        }

        let mut selection = ClipboardSelection::default();
        let mut total_bytes = 0usize;
        for (index, mime_type) in mime_types.into_iter().enumerate() {
            let payload = self.read_mime_payload(&mime_type, index > 0).await?;
            total_bytes = total_bytes.saturating_add(payload.len());
            if total_bytes > MAX_CLIPBOARD_TOTAL_BYTES {
                bail!("Clipboard contents exceed the 64 MiB restore limit");
            }
            selection.formats.push((mime_type, payload));
        }
        Ok(selection)
    }

    pub async fn replace_selection(&mut self, selection: ClipboardSelection) -> Result<()> {
        let is_empty = selection.is_empty();
        if is_empty {
            // First take ownership with a sentinel MIME type outside Mutter's
            // MetaClipboardManager allowlist (`text/plain*`, `image/*`) so
            // Mutter drops its saved copy of the temporary paste text, then
            // unset the selection completely.
            self.selection
                .send_replace(Arc::new(ClipboardSelection::default()));
            let _ = self
                .set_selection_and_await_owner(vec![EMPTY_CLIPBOARD_CLEAR_MIME.to_string()])
                .await;
            self.drain_owner_updates();
            let proxy = clipboard_proxy(&self.connection).await?;
            let empty_options: HashMap<&str, Value<'_>> = HashMap::new();
            tokio::time::timeout(
                CLIPBOARD_CALL_TIMEOUT,
                proxy.call::<_, _, ()>("SetSelection", &(&self.session_handle, empty_options)),
            )
            .await
            .context("Clipboard SetSelection unset call timed out")?
            .context("Clipboard SetSelection unset call failed")?;
            self.owner = false;
            self.latest_owner = Some(ClipboardOwner::default());
            let _ = tokio::time::timeout(CLEAR_OWNER_SETTLE_TIMEOUT, async {
                while let Some(owner) = self.owner_changed.recv().await {
                    self.owner = owner.session_is_owner;
                    self.latest_owner = Some(owner.clone());
                    if !owner.session_is_owner {
                        break;
                    }
                }
            })
            .await;
            return Ok(());
        }

        let mime_types = selection.mime_types();
        self.selection.send_replace(Arc::new(selection));
        self.set_selection_and_await_owner(mime_types).await
    }

    async fn set_selection_and_await_owner(&mut self, mime_types: Vec<String>) -> Result<()> {
        let mut options: HashMap<&str, Value<'_>> = HashMap::new();
        options.insert("mime_types", Value::from(mime_types));
        self.drain_owner_updates();

        let proxy = clipboard_proxy(&self.connection).await?;
        tokio::time::timeout(
            CLIPBOARD_CALL_TIMEOUT,
            proxy.call::<_, _, ()>("SetSelection", &(&self.session_handle, options)),
        )
        .await
        .context("Clipboard SetSelection call timed out")?
        .context("Clipboard SetSelection call failed")?;

        // xdg-desktop-portal completes SetSelection before forwarding to the
        // backend. Wait for SelectionOwnerChanged(session_is_owner=true) so the
        // compositor holds the new clipboard before the paste chord fires.
        let deadline = tokio::time::Instant::now() + CLIPBOARD_CALL_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                bail!("Clipboard SetSelection ownership confirmation timed out");
            }
            match tokio::time::timeout(remaining, self.owner_changed.recv()).await {
                Ok(Some(owner)) => {
                    self.owner = owner.session_is_owner;
                    self.latest_owner = Some(owner.clone());
                    if owner.session_is_owner {
                        return Ok(());
                    }
                }
                Ok(None) => bail!("Clipboard owner notification stream closed"),
                Err(_) => bail!("Clipboard SetSelection ownership confirmation timed out"),
            }
        }
    }

    pub fn selection_still_owned(&mut self) -> bool {
        self.drain_owner_updates();
        self.owner
    }

    pub async fn close(self) {
        self.owner_task.abort();
        self.transfer_task.abort();
        let _ = tokio::time::timeout(
            RELEASE_TIMEOUT,
            close_portal_session(&self.connection, &self.session_handle),
        )
        .await;
    }
}

pub async fn start_portal_keyboard_clipboard_session() -> Result<PortalClipboardSession> {
    hydrate_session_bus_env();

    let connection = Connection::session()
        .await
        .context("failed to connect to session bus for remote desktop clipboard portal")?;
    let persistence = remote_desktop_persistence(&connection).await;
    let permit = claim_restore_permit(&persistence, PortalDeviceKind::KeyboardClipboard).await;
    start_portal_keyboard_clipboard_session_on(connection, permit, true).await
}

async fn start_portal_keyboard_clipboard_session_on(
    connection: Connection,
    permit: Option<RestorePermit>,
    connect_eis: bool,
) -> Result<PortalClipboardSession> {
    let session_handle = create_remote_desktop_session(&connection).await?;
    let mut cleanup = PortalSessionCleanup::new(connection.clone(), session_handle.clone());
    select_keyboard_devices(&connection, &session_handle, permit.as_ref()).await?;

    let clipboard = clipboard_proxy(&connection).await?;
    let owner_stream = clipboard
        .receive_signal("SelectionOwnerChanged")
        .await
        .context("failed to subscribe to Clipboard SelectionOwnerChanged")?;
    let transfer_stream = clipboard
        .receive_signal("SelectionTransfer")
        .await
        .context("failed to subscribe to Clipboard SelectionTransfer")?;

    let (owner_sender, owner_changed) = mpsc::unbounded_channel();
    let owner_task = tokio::spawn(forward_owner_changes(
        owner_stream,
        session_handle.clone(),
        owner_sender,
    ));

    let (selection, receiver) = watch::channel(Arc::new(ClipboardSelection::default()));
    let transfer_task = tokio::spawn(serve_clipboard_transfers(
        connection.clone(),
        session_handle.clone(),
        transfer_stream,
        receiver,
    ));

    let request_options: HashMap<&str, Value<'_>> = HashMap::new();
    let request_result = tokio::time::timeout(
        CLIPBOARD_CALL_TIMEOUT,
        clipboard.call::<_, _, ()>("RequestClipboard", &(&session_handle, request_options)),
    )
    .await
    .context("Clipboard RequestClipboard call timed out")
    .and_then(|res| res.context("Clipboard RequestClipboard call failed"));
    if let Err(error) = request_result {
        owner_task.abort();
        transfer_task.abort();
        return Err(error);
    }

    let start_result = start_remote_desktop_session(&connection, &session_handle).await;
    let (devices, _, restore_token, clipboard_enabled) = match start_result {
        Ok(values) => values,
        Err(error) => {
            owner_task.abort();
            transfer_task.abort();
            return Err(error);
        }
    };

    let granted = devices & DEVICE_KEYBOARD != 0 && clipboard_enabled;
    let restore_token = restore_token_to_keep(granted, restore_token);
    commit_restore_permit(permit, restore_token).await;

    if devices & DEVICE_KEYBOARD == 0 {
        owner_task.abort();
        transfer_task.abort();
        bail!("remote desktop portal session started without keyboard access");
    }
    if !clipboard_enabled {
        owner_task.abort();
        transfer_task.abort();
        bail!("remote desktop portal session started without clipboard access");
    }

    let ei = if connect_eis {
        match portal_eis_input(&connection, &session_handle, DeviceKind::Keyboard, false).await {
            Ok(ei) => ei,
            Err(error) => {
                owner_task.abort();
                transfer_task.abort();
                return Err(error);
            }
        }
    } else {
        None
    };

    cleanup.disarm();
    let keyboard = PortalKeyboardSession {
        ei,
        connection: connection.clone(),
        session_handle: session_handle.clone(),
        input_lock: portal_input_lock(),
        valid: Arc::new(AtomicBool::new(true)),
    };
    Ok(PortalClipboardSession {
        keyboard,
        connection,
        session_handle,
        owner_changed,
        owner_task,
        transfer_task,
        selection,
        latest_owner: None,
        initial_owner_checked: false,
        owner: false,
    })
}

async fn clipboard_proxy<'a>(connection: &'a Connection) -> Result<Proxy<'a>> {
    Proxy::new(
        connection,
        PORTAL_DESKTOP_SERVICE,
        PORTAL_DESKTOP_PATH,
        PORTAL_CLIPBOARD_INTERFACE,
    )
    .await
    .context("failed to create remote desktop clipboard portal proxy")
}

async fn forward_owner_changes(
    mut stream: SignalStream<'static>,
    session_handle: OwnedObjectPath,
    sender: mpsc::UnboundedSender<ClipboardOwner>,
) {
    while let Some(signal) = stream.next().await {
        let Ok((signal_session, options)) = signal
            .body()
            .deserialize::<(OwnedObjectPath, HashMap<String, OwnedValue>)>()
        else {
            continue;
        };
        if signal_session != session_handle {
            continue;
        }
        let owner = parse_owner_options(&options);
        if sender.send(owner).is_err() {
            break;
        }
    }
}

async fn serve_clipboard_transfers(
    connection: Connection,
    session_handle: OwnedObjectPath,
    mut stream: SignalStream<'static>,
    selection: watch::Receiver<Arc<ClipboardSelection>>,
) {
    while let Some(signal) = stream.next().await {
        let Ok((signal_session, mime_type, serial)) = signal
            .body()
            .deserialize::<(OwnedObjectPath, String, u32)>()
        else {
            continue;
        };
        if signal_session != session_handle {
            continue;
        }
        let payload = selection.borrow().payload(&mime_type).map(<[u8]>::to_vec);
        let connection = connection.clone();
        let session_handle = session_handle.clone();
        tokio::spawn(async move {
            let Ok(proxy) = clipboard_proxy(&connection).await else {
                return;
            };
            let fd: zbus::zvariant::OwnedFd = match tokio::time::timeout(
                CLIPBOARD_CALL_TIMEOUT,
                proxy.call("SelectionWrite", &(&session_handle, serial)),
            )
            .await
            {
                Ok(Ok(fd)) => fd,
                _ => return,
            };
            let fd: std::os::fd::OwnedFd = fd.into();
            let blocking = set_blocking(&fd).is_ok();
            let file = std::fs::File::from(fd);
            let success = blocking
                && match payload {
                    Some(payload) => matches!(
                        tokio::time::timeout(
                            CLIPBOARD_CALL_TIMEOUT,
                            tokio::task::spawn_blocking(move || {
                                let mut file = file;
                                let result = file.write_all(&payload);
                                drop(file);
                                result
                            }),
                        )
                        .await,
                        Ok(Ok(Ok(())))
                    ),
                    None => {
                        drop(file);
                        false
                    }
                };
            let _ = tokio::time::timeout(
                CLIPBOARD_CALL_TIMEOUT,
                proxy.call::<_, _, ()>("SelectionWriteDone", &(&session_handle, serial, success)),
            )
            .await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::os::unix::net::UnixStream;
    use std::sync::Mutex as StdMutex;
    use zbus::object_server::SignalEmitter;

    #[test]
    fn text_selection_offers_utf8_plain_and_xwayland_utf8_string() {
        let selection = ClipboardSelection::text("你好 Hello");
        assert_eq!(
            selection.mime_types(),
            ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"]
        );
        for mime_type in selection.mime_types() {
            assert_eq!(selection.payload(&mime_type), Some("你好 Hello".as_bytes()));
        }
        assert_eq!(selection.payload("text/html"), None);
    }

    #[test]
    fn filter_readable_mime_types_skips_icccm_meta_targets_and_duplicates() {
        let input = vec![
            "TARGETS".to_string(),
            "text/html".to_string(),
            "TIMESTAMP".to_string(),
            "text/plain;charset=utf-8".to_string(),
            "text/html".to_string(),
            "".to_string(),
            "SAVE_TARGETS".to_string(),
        ];
        assert_eq!(
            filter_readable_mime_types(&input),
            vec![
                "text/html".to_string(),
                "text/plain;charset=utf-8".to_string()
            ]
        );
    }

    #[derive(Default)]
    struct FakePortalState {
        clipboard_requested: bool,
        grant_clipboard: bool,
        initial_formats: Vec<(String, Vec<u8>)>,
        fail_first_second_read: bool,
        read_calls: usize,
        set_selection_calls: Vec<Vec<String>>,
        keycodes: Vec<(i32, u32)>,
        transfers: BTreeMap<u32, UnixStream>,
        write_done: Vec<(u32, bool)>,
        closed_sessions: usize,
    }

    struct FakeRequest;
    #[zbus::interface(name = "org.freedesktop.portal.Request")]
    impl FakeRequest {
        #[zbus(signal)]
        async fn response(
            emitter: &SignalEmitter<'_>,
            response: u32,
            results: HashMap<String, Value<'_>>,
        ) -> zbus::Result<()>;
    }

    struct FakeSession {
        state: Arc<StdMutex<FakePortalState>>,
    }
    #[zbus::interface(name = "org.freedesktop.portal.Session")]
    impl FakeSession {
        async fn close(&self) {
            self.state.lock().unwrap().closed_sessions += 1;
        }
    }

    struct FakeRemoteDesktop {
        state: Arc<StdMutex<FakePortalState>>,
    }

    #[zbus::interface(name = "org.freedesktop.portal.RemoteDesktop")]
    impl FakeRemoteDesktop {
        #[zbus(property)]
        async fn version(&self) -> u32 {
            2
        }

        async fn create_session(
            &self,
            options: HashMap<String, OwnedValue>,
            #[zbus(connection)] conn: &Connection,
            #[zbus(header)] header: zbus::message::Header<'_>,
        ) -> zbus::fdo::Result<OwnedObjectPath> {
            let sender = header.sender().unwrap().as_str();
            let sender_part = sender.trim_start_matches(':').replace('.', "_");
            let handle_token =
                String::try_from(options["handle_token"].try_clone().unwrap()).unwrap();
            let session_token =
                String::try_from(options["session_handle_token"].try_clone().unwrap()).unwrap();
            let req_path = OwnedObjectPath::try_from(format!(
                "/org/freedesktop/portal/desktop/request/{sender_part}/{handle_token}"
            ))
            .unwrap();
            let session_path = OwnedObjectPath::try_from(format!(
                "/org/freedesktop/portal/desktop/session/{sender_part}/{session_token}"
            ))
            .unwrap();
            conn.object_server()
                .at(
                    session_path.clone(),
                    FakeSession {
                        state: Arc::clone(&self.state),
                    },
                )
                .await
                .unwrap();
            let conn = conn.clone();
            let req_clone = req_path.clone();
            tokio::spawn(async move {
                let emitter = SignalEmitter::new(&conn, req_clone).unwrap();
                let mut results: HashMap<String, Value<'_>> = HashMap::new();
                results.insert(
                    "session_handle".to_string(),
                    Value::from(session_path.as_str()),
                );
                let _ = FakeRequest::response(&emitter, 0, results).await;
            });
            Ok(req_path)
        }

        async fn select_devices(
            &self,
            _session: OwnedObjectPath,
            options: HashMap<String, OwnedValue>,
            #[zbus(connection)] conn: &Connection,
            #[zbus(header)] header: zbus::message::Header<'_>,
        ) -> zbus::fdo::Result<OwnedObjectPath> {
            let sender = header.sender().unwrap().as_str();
            let sender_part = sender.trim_start_matches(':').replace('.', "_");
            let handle_token =
                String::try_from(options["handle_token"].try_clone().unwrap()).unwrap();
            let req_path = OwnedObjectPath::try_from(format!(
                "/org/freedesktop/portal/desktop/request/{sender_part}/{handle_token}"
            ))
            .unwrap();
            let conn = conn.clone();
            let req_clone = req_path.clone();
            tokio::spawn(async move {
                let emitter = SignalEmitter::new(&conn, req_clone).unwrap();
                let _ = FakeRequest::response(&emitter, 0, HashMap::new()).await;
            });
            Ok(req_path)
        }

        async fn start(
            &self,
            session: OwnedObjectPath,
            _parent_window: String,
            options: HashMap<String, OwnedValue>,
            #[zbus(connection)] conn: &Connection,
            #[zbus(header)] header: zbus::message::Header<'_>,
        ) -> zbus::fdo::Result<OwnedObjectPath> {
            let sender = header.sender().unwrap().as_str();
            let sender_part = sender.trim_start_matches(':').replace('.', "_");
            let handle_token =
                String::try_from(options["handle_token"].try_clone().unwrap()).unwrap();
            let req_path = OwnedObjectPath::try_from(format!(
                "/org/freedesktop/portal/desktop/request/{sender_part}/{handle_token}"
            ))
            .unwrap();
            let (clipboard_enabled, initial_mimes) = {
                let state = self.state.lock().unwrap();
                let enabled = state.clipboard_requested && state.grant_clipboard;
                let mimes: Vec<String> = state
                    .initial_formats
                    .iter()
                    .map(|(m, _)| m.clone())
                    .collect();
                (enabled, mimes)
            };
            let conn = conn.clone();
            let req_clone = req_path.clone();
            tokio::spawn(async move {
                if clipboard_enabled && !initial_mimes.is_empty() {
                    let cb_emitter = SignalEmitter::new(&conn, PORTAL_DESKTOP_PATH).unwrap();
                    let mut opts: HashMap<String, Value<'_>> = HashMap::new();
                    opts.insert("mime_types".to_string(), Value::from(initial_mimes));
                    opts.insert("session_is_owner".to_string(), Value::from(false));
                    let _ =
                        FakeClipboard::selection_owner_changed(&cb_emitter, session, opts).await;
                }
                let emitter = SignalEmitter::new(&conn, req_clone).unwrap();
                let mut results: HashMap<String, Value<'_>> = HashMap::new();
                results.insert("devices".to_string(), Value::from(DEVICE_KEYBOARD));
                results.insert(
                    "clipboard_enabled".to_string(),
                    Value::from(clipboard_enabled),
                );
                let _ = FakeRequest::response(&emitter, 0, results).await;
            });
            Ok(req_path)
        }

        async fn notify_keyboard_keycode(
            &self,
            _session: OwnedObjectPath,
            _options: HashMap<String, OwnedValue>,
            keycode: i32,
            state: u32,
        ) {
            self.state.lock().unwrap().keycodes.push((keycode, state));
        }
    }

    struct FakeClipboard {
        state: Arc<StdMutex<FakePortalState>>,
    }

    #[zbus::interface(name = "org.freedesktop.portal.Clipboard")]
    impl FakeClipboard {
        #[zbus(property)]
        async fn version(&self) -> u32 {
            1
        }

        async fn request_clipboard(
            &self,
            _session: OwnedObjectPath,
            _options: HashMap<String, OwnedValue>,
        ) {
            self.state.lock().unwrap().clipboard_requested = true;
        }

        async fn set_selection(
            &self,
            session: OwnedObjectPath,
            options: HashMap<String, OwnedValue>,
            #[zbus(connection)] conn: &Connection,
        ) -> zbus::fdo::Result<()> {
            let mimes = options
                .get("mime_types")
                .map(owned_string_list)
                .transpose()
                .map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?
                .unwrap_or_default();
            if options.contains_key("mime_types") && mimes.is_empty() {
                return Err(zbus::fdo::Error::Failed(
                    "No mime types in mime types list".to_string(),
                ));
            }
            self.state
                .lock()
                .unwrap()
                .set_selection_calls
                .push(mimes.clone());
            let conn = conn.clone();
            tokio::spawn(async move {
                let emitter = SignalEmitter::new(&conn, PORTAL_DESKTOP_PATH).unwrap();
                let mut opts: HashMap<String, Value<'_>> = HashMap::new();
                if !mimes.is_empty() {
                    opts.insert("mime_types".to_string(), Value::from(mimes));
                    opts.insert("session_is_owner".to_string(), Value::from(true));
                } else {
                    opts.insert("session_is_owner".to_string(), Value::from(false));
                }
                let _ = FakeClipboard::selection_owner_changed(&emitter, session, opts).await;
            });
            Ok(())
        }

        async fn selection_read(
            &self,
            _session: OwnedObjectPath,
            mime_type: String,
        ) -> zbus::fdo::Result<zbus::zvariant::OwnedFd> {
            let payload = {
                let mut state = self.state.lock().unwrap();
                state.read_calls += 1;
                if state.fail_first_second_read && state.read_calls == 2 {
                    state.fail_first_second_read = false;
                    return Err(zbus::fdo::Error::Failed("Internal error".to_string()));
                }
                state
                    .initial_formats
                    .iter()
                    .find(|(m, _)| m == &mime_type)
                    .map(|(_, bytes)| bytes.clone())
                    .ok_or_else(|| zbus::fdo::Error::Failed(format!("missing {mime_type}")))?
            };
            let (reader, mut writer) = UnixStream::pair().unwrap();
            let _ = reader.set_nonblocking(true);
            writer.write_all(&payload).unwrap();
            drop(writer);
            let fd: std::os::fd::OwnedFd = reader.into();
            Ok(fd.into())
        }

        async fn selection_write(
            &self,
            _session: OwnedObjectPath,
            serial: u32,
        ) -> zbus::fdo::Result<zbus::zvariant::OwnedFd> {
            let (reader, writer) = UnixStream::pair().unwrap();
            let _ = writer.set_nonblocking(true);
            self.state.lock().unwrap().transfers.insert(serial, reader);
            let fd: std::os::fd::OwnedFd = writer.into();
            Ok(fd.into())
        }

        async fn selection_write_done(
            &self,
            _session: OwnedObjectPath,
            serial: u32,
            success: bool,
        ) {
            self.state
                .lock()
                .unwrap()
                .write_done
                .push((serial, success));
        }

        #[zbus(signal)]
        async fn selection_owner_changed(
            emitter: &SignalEmitter<'_>,
            session_handle: OwnedObjectPath,
            options: HashMap<String, Value<'_>>,
        ) -> zbus::Result<()>;

        #[zbus(signal)]
        async fn selection_transfer(
            emitter: &SignalEmitter<'_>,
            session_handle: OwnedObjectPath,
            mime_type: String,
            serial: u32,
        ) -> zbus::Result<()>;
    }

    async fn start_fake_portal(
        state: Arc<StdMutex<FakePortalState>>,
    ) -> (crate::test_sockets::PrivateBus, Connection, Connection) {
        let bus = crate::test_sockets::PrivateBus::start("cul-cb");
        let portal_conn = zbus::connection::Builder::address(bus.address())
            .unwrap()
            .name(PORTAL_DESKTOP_SERVICE)
            .unwrap()
            .serve_at(
                PORTAL_DESKTOP_PATH,
                FakeRemoteDesktop {
                    state: Arc::clone(&state),
                },
            )
            .unwrap()
            .serve_at(
                PORTAL_DESKTOP_PATH,
                FakeClipboard {
                    state: Arc::clone(&state),
                },
            )
            .unwrap()
            .build()
            .await
            .unwrap();
        let client_conn = zbus::connection::Builder::address(bus.address())
            .unwrap()
            .build()
            .await
            .unwrap();
        (bus, portal_conn, client_conn)
    }

    #[tokio::test]
    async fn fake_portal_preserves_rich_clipboard_retries_parallel_read_and_serves_transfers() {
        let state = Arc::new(StdMutex::new(FakePortalState {
            grant_clipboard: true,
            fail_first_second_read: true,
            initial_formats: vec![
                ("text/html".to_string(), b"<b>keep</b>".to_vec()),
                ("text/plain;charset=utf-8".to_string(), b"keep".to_vec()),
            ],
            ..Default::default()
        }));
        let (_bus, portal_conn, client_conn) = start_fake_portal(Arc::clone(&state)).await;
        let mut session = start_portal_keyboard_clipboard_session_on(client_conn, None, false)
            .await
            .unwrap();

        let previous = session.read_current_selection().await.unwrap();
        assert_eq!(previous.payload("text/html"), Some(&b"<b>keep</b>"[..]));
        assert_eq!(
            previous.payload("text/plain;charset=utf-8"),
            Some(&b"keep"[..])
        );
        // 1st read + failed 2nd read + retried 2nd read = 3 calls
        assert_eq!(state.lock().unwrap().read_calls, 3);

        session
            .replace_selection(ClipboardSelection::text("你好"))
            .await
            .unwrap();
        assert!(session.selection_still_owned());

        // Restore previous rich clipboard, then request a transfer via the portal.
        session.replace_selection(previous.clone()).await.unwrap();
        assert!(session.selection_still_owned());

        let emitter = SignalEmitter::new(&portal_conn, PORTAL_DESKTOP_PATH).unwrap();
        FakeClipboard::selection_transfer(
            &emitter,
            session.session_handle.clone(),
            "text/html".to_string(),
            42,
        )
        .await
        .unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        let mut received = Vec::new();
        loop {
            if let Some(mut stream) = state.lock().unwrap().transfers.remove(&42) {
                stream.read_to_end(&mut received).unwrap();
                break;
            }
            assert!(tokio::time::Instant::now() < deadline, "transfer timed out");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(received, b"<b>keep</b>");

        // Second paste reuses cached session without calling SelectionRead on own selection.
        let calls_before = state.lock().unwrap().read_calls;
        let second_prev = session.read_current_selection().await.unwrap();
        assert_eq!(second_prev, previous);
        assert_eq!(state.lock().unwrap().read_calls, calls_before);

        // External clipboard owner takes over -> selection_still_owned flips to false.
        let mut ext_opts: HashMap<String, Value<'_>> = HashMap::new();
        ext_opts.insert("mime_types".to_string(), Value::from(vec!["text/plain"]));
        ext_opts.insert("session_is_owner".to_string(), Value::from(false));
        FakeClipboard::selection_owner_changed(&emitter, session.session_handle.clone(), ext_opts)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!session.selection_still_owned());

        session.close().await;
        assert_eq!(state.lock().unwrap().closed_sessions, 1);
    }

    #[tokio::test]
    async fn fake_portal_handles_empty_clipboard_and_unsets_cleanly() {
        let state = Arc::new(StdMutex::new(FakePortalState {
            grant_clipboard: true,
            initial_formats: Vec::new(),
            ..Default::default()
        }));
        let (_bus, _portal_conn, client_conn) = start_fake_portal(Arc::clone(&state)).await;
        let mut session = start_portal_keyboard_clipboard_session_on(client_conn, None, false)
            .await
            .unwrap();

        let previous = session.read_current_selection().await.unwrap();
        assert!(previous.is_empty());

        session
            .replace_selection(ClipboardSelection::text("你好"))
            .await
            .unwrap();
        assert!(session.selection_still_owned());

        // Restoring an empty clipboard clears Mutter's clipboard manager cache
        // with the sentinel MIME type, then unsets selection (empty options).
        session.replace_selection(previous).await.unwrap();
        assert!(!session.selection_still_owned());
        let calls = state.lock().unwrap().set_selection_calls.clone();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[1], vec![EMPTY_CLIPBOARD_CLEAR_MIME.to_string()]);
        assert!(calls[2].is_empty());

        session.close().await;
    }

    #[tokio::test]
    async fn fake_portal_rejects_session_without_clipboard_grant() {
        let state = Arc::new(StdMutex::new(FakePortalState {
            grant_clipboard: false,
            ..Default::default()
        }));
        let (_bus, _portal_conn, client_conn) = start_fake_portal(Arc::clone(&state)).await;
        let error = start_portal_keyboard_clipboard_session_on(client_conn, None, false)
            .await
            .err()
            .expect("missing clipboard grant must fail");
        assert!(
            error.to_string().contains("without clipboard access"),
            "{error:#}"
        );
    }
}
