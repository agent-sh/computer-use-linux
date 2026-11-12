//! Read-only preflight for Mutter's portal keysym route. No surfaces or input
//! devices are created. A background Wayland client cannot establish the active
//! layout group, so every configured group must support the complete text.

use anyhow::{bail, Context, Result};
use kbvm::{builder::Redirect, xkb::diagnostic::WriteToLog};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    env,
    ffi::CStr,
    fs::File,
    io,
    os::fd::{AsRawFd, OwnedFd},
    os::unix::fs::FileExt,
    path::PathBuf,
    time::{Duration, Instant},
};
use wayland_client::{
    protocol::{wl_callback, wl_keyboard, wl_registry, wl_seat},
    Connection, Dispatch, QueueHandle, WEnum,
};

const READ_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_KEYMAP_BYTES: u32 = 1024 * 1024;

pub(crate) async fn validate_gnome_text(text: &str, keysyms: &[i32]) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    let text = text.to_string();
    let keysyms = keysyms.to_vec();
    tokio::task::spawn_blocking(move || {
        let deadline = Instant::now() + READ_TIMEOUT;
        let bytes = read_keymap(deadline)?;
        let keymap = Keymap::parse(bytes)?;
        let groups = keymap.first_levels(deadline)?;
        validate_groups(&text, &keysyms, &groups)
    })
    .await
    .context("keyboard keymap preflight failed")?
}

#[derive(Default)]
struct Reader {
    registry_done: bool,
    seats: Vec<wl_seat::WlSeat>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    keymap: Option<Result<Vec<u8>>>,
}

fn read_keymap(deadline: Instant) -> Result<Vec<u8>> {
    // Do not consume WAYLAND_SOCKET: this is a separate, short-lived read-only
    // connection, not the owning client's inherited connection.
    let display = PathBuf::from(
        env::var_os("WAYLAND_DISPLAY")
            .context("WAYLAND_DISPLAY is missing; cannot verify the keyboard keymap")?,
    );
    let path = if display.is_absolute() {
        display
    } else {
        let runtime = PathBuf::from(
            env::var_os("XDG_RUNTIME_DIR")
                .context("XDG_RUNTIME_DIR is missing; cannot verify the keyboard keymap")?,
        );
        if !runtime.is_absolute() {
            bail!("XDG_RUNTIME_DIR must be absolute to verify the keyboard keymap");
        }
        runtime.join(display)
    };
    let stream = crate::x11_display::connect_unix(
        path.to_str().context("Wayland socket path is not UTF-8")?,
        deadline,
    )
    .context("cannot connect to Wayland to verify the keyboard keymap")?;
    let conn =
        Connection::from_socket(stream).context("cannot initialize Wayland keymap reader")?;
    let mut queue = conn.new_event_queue::<Reader>();
    let qh = queue.handle();
    let _registry = conn.display().get_registry(&qh, ());
    conn.display().sync(&qh, ());
    let mut state = Reader::default();
    loop {
        check_deadline(deadline)?;
        queue.dispatch_pending(&mut state)?;
        if state.registry_done {
            if state.seats.len() != 1 {
                bail!("cannot verify a keyboard keymap without exactly one Wayland seat; use set_value on an editable field");
            }
            if let Some(keymap) = state.keymap.take() {
                return keymap;
            }
        }
        // The messages are tiny. A full outgoing socket fails promptly instead
        // of waiting beyond the shared deadline.
        queue
            .flush()
            .context("cannot request the Wayland keyboard keymap")?;
        if let Some(guard) = queue.prepare_read() {
            let mut fd = libc::pollfd {
                fd: guard.connection_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let remaining = deadline.saturating_duration_since(Instant::now());
            let millis = i32::try_from(remaining.as_millis())
                .unwrap_or(i32::MAX)
                .max(1);
            // SAFETY: fd describes one live Wayland connection for this call.
            let status = unsafe { libc::poll(&mut fd, 1, millis) };
            if status == 0 {
                bail!("keyboard keymap read timed out; no text was sent");
            }
            if status < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error).context("keyboard keymap read failed");
            }
            guard
                .read()
                .context("Wayland keyboard keymap read failed")?;
        }
    }
}

fn check_deadline(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        bail!("keyboard keymap preflight timed out; no text was sent");
    }
    Ok(())
}

impl Dispatch<wl_registry::WlRegistry, ()> for Reader {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            // Two seats are enough to reject ambiguous device selection.
            if interface == "wl_seat" && state.seats.len() < 2 {
                state
                    .seats
                    .push(registry.bind(name, version.min(7), qh, ()));
            }
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for Reader {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.registry_done = true;
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for Reader {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        {
            if state.keyboard.is_none() && capabilities.contains(wl_seat::Capability::Keyboard) {
                state.keyboard = Some(seat.get_keyboard(qh, ()));
            }
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for Reader {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Keymap { format, fd, size } = event {
            state.keymap = Some(
                if format == WEnum::Value(wl_keyboard::KeymapFormat::XkbV1) {
                    read_keymap_file(fd, size)
                } else {
                    Err(anyhow::anyhow!(
                        "Wayland keyboard keymap is not XKB text format"
                    ))
                },
            );
        }
    }
}

fn read_keymap_file(fd: OwnedFd, size: u32) -> Result<Vec<u8>> {
    if size == 0 || size > MAX_KEYMAP_BYTES {
        bail!("Wayland keyboard keymap exceeds the 1 MiB read limit or is empty");
    }
    let file = File::from(fd);
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() < u64::from(size) {
        bail!("Wayland keyboard keymap is not a complete regular file");
    }
    let mut bytes = vec![0; size as usize];
    file.read_exact_at(&mut bytes, 0)
        .context("Wayland keyboard keymap was truncated")?;
    Ok(bytes)
}

struct Keymap {
    map: kbvm::xkb::Keymap,
    text: KeymapText,
}

impl Keymap {
    fn parse(bytes: Vec<u8>) -> Result<Self> {
        let text = CStr::from_bytes_with_nul(&bytes)
            .context("keyboard keymap has invalid string framing")?
            .to_bytes();
        // The compositor's keymap is self-contained, so no include paths
        // (XKB_CONTEXT_NO_DEFAULT_INCLUDES in libxkbcommon).
        let mut context = kbvm::xkb::Context::builder();
        context.enable_default_includes(false);
        context.enable_environment(false);
        let map = context
            .build()
            .keymap_from_bytes(WriteToLog, None, text)
            .context("cannot parse the Wayland keyboard keymap")?;
        let text = KeymapText::scan(text)?;
        // kbvm only drops trailing layouts that it found empty, so its
        // layouts must be a prefix of libxkbcommon's for the same key.
        for key in map.keys() {
            let code = key.keycode().raw();
            let layouts = text.keys.get(&code).map_or(0, KeyLayouts::count);
            if text.names.get(key.name()) != Some(&code) || key.groups().count() > layouts {
                bail!("{KEYMAP_TEXT_ERROR}");
            }
        }
        Ok(Self { map, text })
    }

    fn first_levels(&self, deadline: Instant) -> Result<Vec<HashMap<u32, u32>>> {
        // Mutter resolves the first matching keycode/level in the active
        // group. Preserve that ordering, including unsupported higher levels.
        // kbvm supplies each layout's levels and keysyms; the keymap text
        // supplies what kbvm drops: the keycode range and every key's layout
        // count, including empty trailing layouts kbvm does not keep.
        let KeymapText { min, max, .. } = self.text;
        let groups = self.text.keys.values().map(KeyLayouts::count).max();
        let groups = groups.unwrap_or(0);
        if groups == 0 || groups > 16 || max > 65535 || min > max {
            bail!("keyboard keymap exceeds supported layout/keycode limits");
        }
        let kbvm_keys: HashMap<_, _> = self
            .map
            .keys()
            .map(|key| (key.keycode().raw(), key))
            .collect();
        // Mutter's legacy lookup excludes the upper bound too.
        let mut keys: Vec<_> = self
            .text
            .keys
            .iter()
            .filter(|(code, _)| (min..max).contains(*code))
            .collect();
        keys.sort_unstable_by_key(|(code, _)| **code);
        let mut result = Vec::new();
        for group in 0..groups {
            let mut first = HashMap::new();
            for (code, layouts) in &keys {
                check_deadline(deadline)?;
                let Some(index) = layouts.layout(group) else {
                    continue;
                };
                // An index past kbvm's layouts is an empty layout.
                let Some(key_group) = kbvm_keys
                    .get(code)
                    .and_then(|key| key.groups().nth(index).flatten())
                else {
                    continue;
                };
                if key_group.levels().count() > 32
                    || key_group
                        .ty()
                        .mappings()
                        .any(|mapping| mapping.level() >= 32)
                {
                    bail!("keyboard keymap exceeds supported level limits");
                }
                for (level, symbols) in key_group.levels().enumerate() {
                    if symbols.symbols().len() > 32 {
                        bail!("keyboard keymap returned an invalid symbol list");
                    }
                    for sym in symbols.symbols() {
                        first.entry(sym.0).or_insert(level as u32);
                    }
                }
            }
            result.push(first);
        }
        Ok(result)
    }
}

const KEYMAP_TEXT_ERROR: &str = "cannot read the keymap's keycodes or layouts";

/// What libxkbcommon reads from the keymap and kbvm does not expose. kbvm
/// keeps only keys named in `xkb_symbols` and drops a key's empty trailing
/// layouts, but libxkbcommon's keycode range and per-key layout counts decide
/// which layout a key uses. Anything a compositor-serialized keymap does not
/// contain fails closed rather than guess.
struct KeymapText {
    /// libxkbcommon's min and max keycode: the extremes of the
    /// `<NAME> = code;` entries in `xkb_keycodes` (it ignores `minimum` and
    /// `maximum`).
    min: u32,
    max: u32,
    /// Key name without brackets, and aliases, to keycode.
    names: HashMap<String, u32>,
    /// The layouts of each key in `xkb_symbols`, by keycode.
    keys: HashMap<u32, KeyLayouts>,
}

/// One key's layouts as libxkbcommon counts them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KeyLayouts {
    /// Bit N: `symbols[N+1]`, `actions[N+1]` or `type[N+1]` defines layout N.
    defined: u32,
    out_of_range: OutOfRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutOfRange {
    Wrap,
    Clamp,
    Redirect(usize),
}

impl KeyLayouts {
    /// xkb_keymap_num_layouts_for_key: up to the highest defined layout.
    fn count(&self) -> usize {
        (u32::BITS - self.defined.leading_zeros()) as usize
    }

    /// The key's layout for keymap layout `group`, or `None` if it has no
    /// symbols there. libxkbcommon's XkbWrapGroupIntoRange, then its fill of
    /// a layout nothing defines from layout 1.
    fn layout(&self, group: usize) -> Option<usize> {
        let count = self.count();
        if count == 0 {
            return None;
        }
        let index = match self.out_of_range {
            _ if group < count => group,
            OutOfRange::Wrap => group % count,
            OutOfRange::Clamp => count - 1,
            OutOfRange::Redirect(target) if target < count => target,
            OutOfRange::Redirect(_) => 0,
        };
        let defined = |index: usize| self.defined & (1 << index) != 0;
        let index = if defined(index) { index } else { 0 };
        defined(index).then_some(index)
    }
}

impl KeymapText {
    fn scan(keymap: &[u8]) -> Result<Self> {
        Self::scan_text(keymap).context(KEYMAP_TEXT_ERROR)
    }

    fn scan_text(keymap: &[u8]) -> Option<Self> {
        // Drop `//` and `#` comments (xkbcommon rejects `/* */`) and string
        // contents, so neither can fake a keycode, brace or separator.
        let mut text = Vec::with_capacity(keymap.len());
        let mut i = 0;
        while i < keymap.len() {
            match keymap[i] {
                b'"' => {
                    i += 1;
                    while i < keymap.len() && keymap[i] != b'"' {
                        i += if keymap[i] == b'\\' { 2 } else { 1 };
                    }
                    text.extend_from_slice(b"\"\"");
                }
                b'#' | b'/' if keymap[i] == b'#' || keymap.get(i + 1) == Some(&b'/') => {
                    while i < keymap.len() && keymap[i] != b'\n' {
                        i += 1;
                    }
                    text.push(b'\n');
                }
                byte => text.push(byte),
            }
            i += 1;
        }
        let text = String::from_utf8(text).ok()?;
        let (names, min, max) = scan_keycodes(section(&text, "xkb_keycodes")?)?;
        let keys = scan_symbols(section(&text, "xkb_symbols")?, &names)?;
        Some(Self {
            min,
            max,
            names,
            keys,
        })
    }
}

/// The body of the first `keyword "name" { ... }` section.
fn section<'a>(text: &'a str, keyword: &str) -> Option<&'a str> {
    let section = &text[text.find(keyword)?..];
    let body = &section[section.find('{')? + 1..];
    Some(&body[..closing(body, '}')?])
}

/// The byte index of the first `close` outside brackets, braces and parens.
fn closing(text: &str, close: char) -> Option<usize> {
    split_top_level(text, close).next().map(str::len)
}

/// Splits at `separator` outside brackets, braces and parentheses.
fn split_top_level(text: &str, separator: char) -> impl Iterator<Item = &str> {
    let mut depth = 0usize;
    text.split(move |c: char| {
        if depth == 0 && c == separator {
            return true;
        }
        match c {
            '[' | '{' | '(' => depth += 1,
            ']' | '}' | ')' => depth = depth.saturating_sub(1),
            _ => {}
        }
        false
    })
}

fn scan_keycodes(body: &str) -> Option<(HashMap<String, u32>, u32, u32)> {
    let mut names = HashMap::new();
    let mut codes = BTreeSet::new();
    let mut aliases = Vec::new();
    for statement in body.split(';').map(str::trim) {
        if statement.is_empty() {
            continue;
        }
        let (name, code) = statement.split_once('=')?;
        let name = name.trim();
        if !name.starts_with('<') {
            match name.split_whitespace().collect::<Vec<_>>()[..] {
                ["alias", alias] => aliases.push((key_name(alias)?, key_name(code.trim())?)),
                ["minimum" | "maximum"] | ["indicator", _] | ["virtual", "indicator", _] => {}
                _ => return None,
            }
            continue;
        }
        let code = code.trim();
        let code = match code.strip_prefix("0x") {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => code.parse().ok()?,
        };
        // A repeated name or code would move a key in libxkbcommon; a
        // serialized keymap has neither, so refuse rather than model it.
        if names.insert(key_name(name)?.to_string(), code).is_some() || !codes.insert(code) {
            return None;
        }
    }
    for (alias, target) in aliases {
        let code = *names.get(target)?;
        if names.insert(alias.to_string(), code).is_some() {
            return None;
        }
    }
    Some((names, *codes.first()?, *codes.last()?))
}

/// `<NAME>` to `NAME`.
fn key_name(text: &str) -> Option<&str> {
    let name = text.strip_prefix('<')?.strip_suffix('>')?;
    (!name.is_empty() && !name.contains(['<', '>'])).then_some(name)
}

fn scan_symbols(body: &str, names: &HashMap<String, u32>) -> Option<HashMap<u32, KeyLayouts>> {
    let mut keys = HashMap::new();
    for statement in split_top_level(body, ';').map(str::trim) {
        if statement.is_empty() {
            continue;
        }
        match statement
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .next()?
        {
            "name" | "modifier_map" | "modmap" | "mod_map" => {}
            "key" => {
                let rest = statement["key".len()..].trim_start();
                let open = rest.find('{')?;
                let code = *names.get(key_name(rest[..open].trim())?)?;
                let key_body = rest[open + 1..].strip_suffix('}')?;
                if keys.insert(code, scan_key(key_body)?).is_some() {
                    return None;
                }
            }
            _ => return None,
        }
    }
    Some(keys)
}

/// A key's layouts, as libxkbcommon's symbols compiler defines them.
fn scan_key(body: &str) -> Option<KeyLayouts> {
    let mut defined = 0u32;
    let mut symbols = 0u32;
    let mut actions = 0u32;
    let mut out_of_range = OutOfRange::Wrap;
    for item in split_top_level(body, ',').map(str::trim) {
        if item.is_empty() {
            continue;
        }
        let (field, value) = match item.split_once('=') {
            _ if item.starts_with('[') => ("symbols", Some(item)),
            Some((field, value)) => (field.trim(), Some(value.trim())),
            None => (item, None),
        };
        let (field, index) = match field.split_once('[') {
            Some((field, index)) => (field.trim(), Some(layout_index(index.strip_suffix(']')?)?)),
            None => (field, None),
        };
        match (field.to_ascii_lowercase().as_str(), index, value) {
            ("type", Some(index), Some(_)) => defined |= 1 << index,
            ("type", None, Some(_)) => {}
            (field @ ("symbols" | "actions"), index, Some(value)) => {
                // A list without an index fills the first layout that lacks
                // that field; a list of calls is actions.
                let field = match (field, value.contains('(')) {
                    ("symbols", false) => &mut symbols,
                    ("actions", _) | (_, true) => &mut actions,
                    _ => return None,
                };
                let index = index.unwrap_or(field.trailing_ones() as usize);
                if index >= 32 {
                    return None;
                }
                *field |= 1 << index;
                defined |= 1 << index;
            }
            ("groupswrap" | "wrapgroups", None, None) => out_of_range = OutOfRange::Wrap,
            ("groupsclamp" | "clampgroups", None, None) => out_of_range = OutOfRange::Clamp,
            ("groupsredirect" | "redirectgroups", None, Some(value)) => {
                out_of_range = OutOfRange::Redirect(layout_index(value)?);
            }
            (
                "repeat" | "repeats" | "repeating" | "virtualmods" | "vmods" | "virtualmodifiers",
                None,
                Some(_),
            ) => {}
            _ => return None,
        }
    }
    Some(KeyLayouts {
        defined,
        out_of_range,
    })
}

/// `1` or `Group1` to layout index 0.
fn layout_index(text: &str) -> Option<usize> {
    let text = text.trim();
    let number = match text.get(..5) {
        Some(prefix) if prefix.eq_ignore_ascii_case("group") => &text[5..],
        _ => text,
    };
    let number: usize = number.parse().ok()?;
    (1..=32).contains(&number).then(|| number - 1)
}

/// The preflight refused the text because a configured layout group cannot
/// type one of its characters. No key was sent. Callers can tell this refusal
/// from a keymap that could not be read with `downcast_ref`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MissingLayoutKeysym {
    pub(crate) ch: char,
    /// One-based layout group number.
    pub(crate) group: usize,
    pub(crate) reason: MissingKeysymReason,
}

/// Why a layout group cannot type a character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MissingKeysymReason {
    /// No key in the group produces it.
    Absent,
    /// A key produces it only at a shift level Mutter cannot synthesize.
    UnreachableLevel,
}

impl MissingLayoutKeysym {
    /// "keyboard layout group 2 lacks U+0068", or the unreachable-level
    /// wording.
    pub(crate) fn summary(&self) -> String {
        match self.reason {
            MissingKeysymReason::Absent => format!(
                "keyboard layout group {} lacks U+{:04X}",
                self.group, self.ch as u32
            ),
            MissingKeysymReason::UnreachableLevel => format!(
                "keyboard layout group {} has U+{:04X} only at a shift level the portal cannot type",
                self.group, self.ch as u32
            ),
        }
    }
}

impl std::fmt::Display for MissingLayoutKeysym {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GNOME portal cannot verify U+{:04X} in keyboard layout group {}: the {}. A background client cannot verify the active group, so every group must support the complete text. No text was sent; use set_value on an editable field.", self.ch as u32, self.group, match self.reason {
            MissingKeysymReason::Absent => "group has no key for it",
            MissingKeysymReason::UnreachableLevel => "group has it only at a shift level the portal cannot type",
        })
    }
}

impl std::error::Error for MissingLayoutKeysym {}

fn validate_groups(text: &str, keysyms: &[i32], groups: &[HashMap<u32, u32>]) -> Result<()> {
    if groups.is_empty() || text.chars().count() != keysyms.len() {
        bail!("cannot verify the complete text against the keyboard keymap");
    }
    for (ch, &keysym) in text.chars().zip(keysyms) {
        for (index, group) in groups.iter().enumerate() {
            let reason = match group.get(&(keysym as u32)) {
                None => Some(MissingKeysymReason::Absent),
                Some(0) => None,
                Some(1) if group.get(&xkeysym::key::Shift_L) == Some(&0) => None,
                Some(2) if group.get(&xkeysym::key::ISO_Level3_Shift) == Some(&0) => None,
                Some(_) => Some(MissingKeysymReason::UnreachableLevel),
            };
            if let Some(reason) = reason {
                return Err(MissingLayoutKeysym {
                    ch,
                    group: index + 1,
                    reason,
                }
                .into());
            }
        }
    }
    Ok(())
}

/// The preflight against a keymap fixture instead of the live Wayland one.
#[cfg(test)]
pub(crate) fn validate_text_with_keymap(keymap: &[u8], text: &str) -> Result<()> {
    let mut bytes = keymap.to_vec();
    bytes.push(0);
    let map = Keymap::parse(bytes)?;
    let groups = map.first_levels(Instant::now() + READ_TIMEOUT)?;
    let keysyms = crate::remote_desktop::keysyms_for_text(text)?;
    validate_groups(text, &keysyms, &groups)
}

#[cfg(test)]
pub(crate) fn debug_key_layout_count(keymap_text: &str, key_name: &str) -> Option<usize> {
    let (names, _, _) = scan_keycodes(section(keymap_text, "xkb_keycodes")?)?;
    let keys = scan_symbols(section(keymap_text, "xkb_symbols")?, &names)?;
    let code = *names.get(key_name)?;
    Some(keys.get(&code).map_or(0, KeyLayouts::count))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Per group, the sorted `0x<keysym> <level>` pairs of `first_levels`.
    fn render_first_levels(groups: &[HashMap<u32, u32>]) -> String {
        let mut out = String::new();
        for (index, group) in groups.iter().enumerate() {
            let mut pairs: Vec<_> = group.iter().collect();
            pairs.sort();
            out += &format!("group {}\n", index + 1);
            for (keysym, level) in pairs {
                out += &format!("0x{keysym:x} {level}\n");
            }
        }
        out
    }

    #[test]
    fn debug_scanner_on_epo_legacy_and_unus_topk() {
        // epo(legacy) empty last layout group case
        let epo = include_bytes!("../tests/fixtures/keyboard-ca-epo.xkb");
        let etext = std::str::from_utf8(epo).unwrap();
        println!(
            "LSGT (empty last group 2): {:?}",
            debug_key_layout_count(etext, "LSGT")
        );
        // <UNUS>/<TOPK> case from group-wrap fixture
        let wrap = include_bytes!("../tests/fixtures/keyboard-group-wrap.xkb");
        let wtext = std::str::from_utf8(wrap).unwrap();
        println!(
            "AC07 (empty last): {:?}",
            debug_key_layout_count(wtext, "AC07")
        );
        println!("AC08 (gap): {:?}", debug_key_layout_count(wtext, "AC08"));
        println!(
            "TOPK (highest with symbols): {:?}",
            debug_key_layout_count(wtext, "TOPK")
        );
        println!(
            "UNUS (declared but unused): {:?}",
            debug_key_layout_count(wtext, "UNUS")
        );
        // regression asserts for the edge cases per ruling (per-key count, fail closed)
        assert_eq!(debug_key_layout_count(wtext, "AC07"), Some(2));
        assert_eq!(debug_key_layout_count(wtext, "AC08"), Some(3));
        assert_eq!(debug_key_layout_count(wtext, "TOPK"), Some(1));
        assert_eq!(debug_key_layout_count(wtext, "UNUS"), Some(0));
        assert_eq!(debug_key_layout_count(etext, "LSGT"), Some(2));
    }

    #[test]

    fn first_levels_match_libxkbcommon_golden() {
        // The goldens are libxkbcommon's output. fr and us-ru are serialized
        // the way Mutter sends them; group-wrap covers per-key group wrap,
        // clamp and redirect and a declared keycode above every key.
        macro_rules! fixture {
            ($name:literal) => {
                (
                    $name,
                    include_bytes!(concat!("../tests/fixtures/keyboard-", $name, ".xkb"))
                        .as_slice(),
                    include_str!(concat!(
                        "../tests/fixtures/keyboard-",
                        $name,
                        ".first-levels"
                    )),
                )
            };
        }
        for (name, keymap, golden) in [
            fixture!("us-de"),
            fixture!("us-il"),
            fixture!("fr"),
            fixture!("us-ru"),
            fixture!("group-wrap"),
            fixture!("us-epo-legacy"),
        ] {
            let mut bytes = keymap.to_vec();

            bytes.push(0);
            let map = Keymap::parse(bytes).expect(name);
            let groups = map.first_levels(Instant::now() + READ_TIMEOUT).unwrap();
            assert_eq!(render_first_levels(&groups), golden, "{name}");
        }
    }

    fn declared_keycode_range(keymap: &[u8]) -> Option<(u32, u32)> {
        let text = std::str::from_utf8(keymap).ok()?;
        let body = section(text, "xkb_keycodes")?;
        scan_keycodes(body).map(|(_, min, max)| (min, max))
    }

    #[test]
    fn keycode_range_comes_from_declared_keycodes_only() {
        let range = |keycodes: &str| {
            declared_keycode_range(
                format!("xkb_keymap {{ xkb_keycodes \"x\" {{ {keycodes} }}; }};").as_bytes(),
            )
        };
        assert_eq!(
            range(
                "minimum = 1; maximum = 1000;\n<A> = 38; // <B> = 900;\n# <C> = 901;\n\
                 alias <D> = <A>; indicator 1 = \"Caps Lock; <E> = 902\";\n\
                 virtual indicator 2 = \"#\"; <F>=0xc8;<G> = 0100;"
            ),
            Some((38, 200))
        );
        // Fail closed: no section, no keycodes, or input libxkbcommon would
        // read differently.
        assert_eq!(declared_keycode_range(b"xkb_keymap { };"), None);
        for keycodes in [
            "",
            "minimum = 8;",
            "<A> = 38; <A> = 300;",
            "<A> = 38; <B> = 38;",
            "<A> = 38; override <B> = 300;",
            "<A> = 38; include \"evdev\";",
            "<A> = 38; /* <B> = 300; */",
            "<A> = -5;",
        ] {
            assert_eq!(range(keycodes), None, "{keycodes}");
        }
    }

    #[test]
    fn real_xkb_fixture_rejects_dead_only_symbols_in_another_group() {
        let mut bytes = include_bytes!("../tests/fixtures/keyboard-us-de.xkb").to_vec();
        bytes.push(0);
        let map = Keymap::parse(bytes).expect("parse the self-contained XKB fixture");
        let groups = map.first_levels(Instant::now() + READ_TIMEOUT).unwrap();
        assert_eq!(groups.len(), 2);
        assert!(validate_groups(
            "aA",
            &[xkeysym::key::a as i32, xkeysym::key::A as i32],
            &groups
        )
        .is_ok());
        assert!(validate_groups(
            "a`",
            &[xkeysym::key::a as i32, xkeysym::key::grave as i32],
            &groups
        )
        .unwrap_err()
        .to_string()
        .contains("group 2"));
        assert!(validate_groups("א", &[xkeysym::key::hebrew_aleph as i32], &groups).is_err());
    }

    #[test]
    fn rejects_complete_text_when_any_group_or_level_is_unsupported() {
        let latin = HashMap::from([
            (xkeysym::key::a, 0),
            (xkeysym::key::asciicircum, 1),
            (xkeysym::key::Shift_L, 0),
        ]);
        let german = HashMap::from([(xkeysym::key::a, 0), (xkeysym::key::dead_circumflex, 0)]);
        assert!(validate_groups(
            "a^",
            &[xkeysym::key::a as i32, xkeysym::key::asciicircum as i32],
            std::slice::from_ref(&latin)
        )
        .is_ok());
        let error = validate_groups(
            "a^",
            &[xkeysym::key::a as i32, xkeysym::key::asciicircum as i32],
            &[latin, german],
        )
        .unwrap_err();
        assert!(error.to_string().contains("U+005E"));
        assert!(error.to_string().contains("group 2"));
        assert_eq!(
            error.downcast_ref::<MissingLayoutKeysym>(),
            Some(&MissingLayoutKeysym {
                ch: '^',
                group: 2,
                reason: MissingKeysymReason::Absent,
            })
        );
        assert!(error.to_string().contains("has no key for it"), "{error}");
        let unsupported_level = HashMap::from([(xkeysym::key::a, 3)]);
        let missing_modifier = HashMap::from([(xkeysym::key::a, 1)]);
        for group in [unsupported_level, missing_modifier] {
            let error = validate_groups("a", &[xkeysym::key::a as i32], &[group]).unwrap_err();
            let missing = *error.downcast_ref::<MissingLayoutKeysym>().unwrap();
            assert_eq!(missing.reason, MissingKeysymReason::UnreachableLevel);
            assert!(
                error.to_string().contains("only at a shift level"),
                "{error}"
            );
            assert!(missing.summary().contains("only at a shift level"));
            assert!(!missing.summary().contains("lacks"));
        }
        // Only a missing keysym is a layout refusal. A keymap the preflight
        // cannot check must not look like one.
        assert!(validate_groups("a", &[xkeysym::key::a as i32], &[])
            .unwrap_err()
            .downcast_ref::<MissingLayoutKeysym>()
            .is_none());
        // Critical empty-final-group case (epo(legacy) <LSGT> group 2)
        let ca_epo = include_bytes!("../tests/fixtures/keyboard-ca-epo.xkb");
        let err = validate_text_with_keymap(ca_epo, "«").unwrap_err();
        assert!(err.to_string().contains("group 2"));
    }
}
