//! Large accessibility trees go to a private file instead of the tool result.
//!
//! A big Electron or browser tree serializes to megabytes, far past what an MCP
//! host shows a model (Codex cuts tool output at 10,000 tokens). Past
//! [`INLINE_TREE_CHARS`] the result keeps only the nodes that fit, and the
//! whole compacted tree is written as JSON to a per-user directory the agent
//! can search. Indices in the file match the cached snapshot, so an
//! `element_index` read from it still works.
//!
//! Files are cleaned up: each spill replaces the server's previous one, the
//! server removes its file when it exits, and every spill sweeps files left by
//! servers that are gone or older than [`MAX_SPILL_AGE`].

use std::fs::{self, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use schemars::JsonSchema;
use serde::Serialize;

use crate::atspi_tree::AccessibilityNode;

/// Largest inline tree, in serialized characters. It keeps a whole
/// `get_app_state` result under a 10,000-token host limit (about 40,000
/// characters) with room for the readiness block and message.
pub(crate) const INLINE_TREE_CHARS: usize = 32_000;
/// A spill file older than this is swept even if its server still runs.
const MAX_SPILL_AGE: Duration = Duration::from_secs(60 * 60);
const SPILL_PREFIX: &str = "tree-";

/// Where the full tree went when it was too large to return inline.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(crate) struct TreeSpill {
    /// JSON array of every compacted node, with the same indices as the cached snapshot.
    pub path: String,
    /// Nodes in the file.
    pub node_count: usize,
    /// File size in bytes.
    pub bytes: usize,
    /// Nodes returned inline, from the start of the tree.
    pub inline_node_count: usize,
}

/// One server's spill files.
pub(crate) struct TreeSpiller {
    dir: Option<PathBuf>,
    current: Mutex<Option<PathBuf>>,
    sequence: AtomicU64,
}

impl Default for TreeSpiller {
    fn default() -> Self {
        Self::new(spill_dir())
    }
}

impl TreeSpiller {
    pub(crate) fn new(dir: Option<PathBuf>) -> Self {
        Self {
            dir,
            current: Mutex::new(None),
            sequence: AtomicU64::new(0),
        }
    }

    /// Fit `nodes` into the inline budget. When they do not fit, write all of
    /// them to a spill file and keep the prefix that fits. Without a writable
    /// spill directory the tree is cut to the prefix and `Err` says why.
    pub(crate) fn fit(
        &self,
        nodes: &mut Vec<AccessibilityNode>,
    ) -> Result<Option<TreeSpill>, (usize, String)> {
        let inline = inline_prefix_len(nodes, INLINE_TREE_CHARS);
        if inline == nodes.len() {
            return Ok(None);
        }
        let result = self.write(nodes).map(|(path, bytes)| TreeSpill {
            path: path.display().to_string(),
            node_count: nodes.len(),
            bytes,
            inline_node_count: inline,
        });
        nodes.truncate(inline);
        result
            .map(Some)
            .map_err(|error| (inline, format!("{error:#}")))
    }

    fn write(&self, nodes: &[AccessibilityNode]) -> io::Result<(PathBuf, usize)> {
        let dir = self.dir.as_deref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "no private directory for spilled trees",
            )
        })?;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        sweep(dir, std::process::id());
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!(
            "{SPILL_PREFIX}{}-{sequence}.json",
            std::process::id()
        ));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer(&mut writer, nodes)?;
        writer.flush()?;
        let bytes = writer.get_ref().metadata()?.len() as usize;
        let previous = self
            .current
            .lock()
            .ok()
            .and_then(|mut current| current.replace(path.clone()));
        if let Some(previous) = previous {
            let _ = fs::remove_file(previous);
        }
        Ok((path, bytes))
    }
}

impl TreeSpiller {
    /// Remove this server's spill file, if any.
    pub(crate) fn remove_current(&self) {
        if let Some(path) = self
            .current
            .lock()
            .ok()
            .and_then(|mut current| current.take())
        {
            let _ = fs::remove_file(path);
        }
    }

    /// Remove files left by servers that are gone, or that are too old.
    pub(crate) fn sweep(&self) {
        if let Some(dir) = &self.dir {
            sweep(dir, std::process::id());
        }
    }
}

impl Drop for TreeSpiller {
    fn drop(&mut self) {
        self.remove_current();
    }
}

/// `$XDG_RUNTIME_DIR/computer-use-linux/trees`: per-user, private, and cleared at
/// logout. Falls back to `~/.cache/computer-use-linux/trees`.
fn spill_dir() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute());
    let base = runtime.or_else(|| {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
    })?;
    Some(base.join("computer-use-linux").join("trees"))
}

/// Number of leading nodes whose JSON array fits in `budget` characters.
pub(crate) fn inline_prefix_len(nodes: &[AccessibilityNode], budget: usize) -> usize {
    let mut used: usize = 2; // the array brackets
    for (index, node) in nodes.iter().enumerate() {
        let size = serde_json::to_string(node).map_or(usize::MAX, |json| json.len())
            + usize::from(index > 0);
        if used.saturating_add(size) > budget {
            return index;
        }
        used += size;
    }
    nodes.len()
}

/// Remove spill files whose server is gone, or that are older than [`MAX_SPILL_AGE`].
fn sweep(dir: &Path, own_pid: u32) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(spill_file_pid) else {
            continue;
        };
        let old = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > MAX_SPILL_AGE);
        if old || (pid != own_pid && !process_alive(pid)) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn spill_file_pid(name: &str) -> Option<u32> {
    name.strip_prefix(SPILL_PREFIX)?
        .strip_suffix(".json")?
        .split_once('-')?
        .0
        .parse()
        .ok()
}

fn process_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(index: u32, name_len: usize) -> AccessibilityNode {
        AccessibilityNode {
            index,
            parent_index: index.checked_sub(1),
            depth: index,
            object_ref: format!(":1.1/org/a11y/atspi/accessible/{index}"),
            role: "paragraph".into(),
            name: Some("x".repeat(name_len)),
            description: None,
            child_count: 0,
            bounds: None,
            states: Vec::new(),
            actions: Vec::new(),
            value: None,
            text: None,
            supports_editable_text: false,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cul-spill-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn files(dir: &Path) -> Vec<PathBuf> {
        fs::read_dir(dir).map_or_else(|_| Vec::new(), |d| d.flatten().map(|e| e.path()).collect())
    }

    #[test]
    fn small_trees_stay_inline_and_write_nothing() {
        let dir = scratch("small");
        let spiller = TreeSpiller::new(Some(dir.clone()));
        let mut nodes = vec![node(0, 10), node(1, 10)];
        assert!(matches!(spiller.fit(&mut nodes), Ok(None)));
        assert_eq!(nodes.len(), 2);
        assert!(files(&dir).is_empty());
    }

    #[test]
    fn large_trees_spill_whole_and_keep_a_prefix_that_fits() {
        let dir = scratch("large");
        let spiller = TreeSpiller::new(Some(dir.clone()));
        let all: Vec<_> = (0..200).map(|i| node(i, 400)).collect();
        let mut nodes = all.clone();
        let spill = spiller.fit(&mut nodes).unwrap().expect("spilled");
        assert_eq!(spill.node_count, 200);
        assert_eq!(spill.inline_node_count, nodes.len());
        assert!(serde_json::to_string(&nodes).unwrap().len() <= INLINE_TREE_CHARS);
        assert!(nodes.len() < 200 && !nodes.is_empty());
        let written: Vec<serde_json::Value> =
            serde_json::from_slice(&fs::read(&spill.path).unwrap()).unwrap();
        assert_eq!(written.len(), 200);
        assert_eq!(
            written[150]["index"], 150,
            "file indices match the snapshot"
        );
        let mode = fs::metadata(&spill.path).unwrap().permissions();
        assert_eq!(
            std::os::unix::fs::PermissionsExt::mode(&mode) & 0o777,
            0o600
        );
        let dir_mode = fs::metadata(&dir).unwrap().permissions();
        assert_eq!(
            std::os::unix::fs::PermissionsExt::mode(&dir_mode) & 0o777,
            0o700
        );
        drop(spiller);
        assert!(
            !Path::new(&spill.path).exists(),
            "the server removes its file on exit"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_new_spill_replaces_the_previous_one() {
        let dir = scratch("replace");
        let spiller = TreeSpiller::new(Some(dir.clone()));
        let first = spiller
            .fit(&mut (0..200).map(|i| node(i, 400)).collect())
            .unwrap()
            .unwrap();
        let second = spiller
            .fit(&mut (0..200).map(|i| node(i, 400)).collect())
            .unwrap()
            .unwrap();
        assert!(!Path::new(&first.path).exists());
        assert_eq!(files(&dir), vec![PathBuf::from(&second.path)]);
        drop(spiller);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn sweep_removes_files_of_servers_that_are_gone() {
        let dir = scratch("sweep");
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .unwrap();
        // pid 0 never names a live server process; other names are left alone.
        let dead = dir.join(format!("{SPILL_PREFIX}0-3.json"));
        let mine = dir.join(format!("{SPILL_PREFIX}{}-0.json", std::process::id()));
        let other = dir.join("notes.txt");
        for path in [&dead, &mine, &other] {
            fs::write(path, "[]").unwrap();
        }
        sweep(&dir, std::process::id());
        assert!(!dead.exists());
        assert!(mine.exists() && other.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn without_a_spill_directory_the_tree_is_cut_and_says_why() {
        let spiller = TreeSpiller::new(None);
        let mut nodes: Vec<_> = (0..200).map(|i| node(i, 400)).collect();
        let (inline, error) = spiller.fit(&mut nodes).unwrap_err();
        assert_eq!(nodes.len(), inline);
        assert!(error.contains("no private directory"));
    }
}
