use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// File watcher that tracks byte offsets for append-only JSONL files.
pub struct FileWatcher {
    pub rx: mpsc::Receiver<notify::Result<Event>>,
    _watcher: RecommendedWatcher,
    /// Byte offset per watched file (for tailing).
    pub offsets: HashMap<PathBuf, u64>,
    /// Debounce: last event time per file.
    last_event: HashMap<PathBuf, Instant>,
    debounce_ms: u64,
}

impl FileWatcher {
    /// Create a new watcher for the given paths.
    pub fn new(watch_paths: &[PathBuf], debounce_ms: u64) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel();

        let mut watcher = RecommendedWatcher::new(tx, Config::default())
            .map_err(|e| format!("Failed to create watcher: {}", e))?;

        for path in watch_paths {
            if path.exists() {
                let mode = if path.is_dir() {
                    RecursiveMode::Recursive
                } else {
                    RecursiveMode::NonRecursive
                };
                watcher
                    .watch(path, mode)
                    .map_err(|e| format!("Failed to watch {}: {}", path.display(), e))?;
                log::info!("Watching: {}", path.display());
            } else {
                log::warn!("Watch path does not exist: {}", path.display());
            }
        }

        Ok(Self {
            rx,
            _watcher: watcher,
            offsets: HashMap::new(),
            last_event: HashMap::new(),
            debounce_ms,
        })
    }

    /// Check for new events. Returns paths of modified files that passed debounce.
    pub fn poll(&mut self) -> Vec<PathBuf> {
        let mut changed = Vec::new();
        let now = Instant::now();

        while let Ok(event_result) = self.rx.try_recv() {
            if let Ok(event) = event_result {
                match event.kind {
                    EventKind::Modify(_) | EventKind::Create(_) => {
                        for path in event.paths {
                            // Only care about .jsonl and .md files
                            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                            if ext != "jsonl" && ext != "md" {
                                continue;
                            }

                            // Retain every event until the file has been quiet.
                            self.last_event.insert(path.clone(), now);
                        }
                    }
                    _ => {}
                }
            }
        }

        self.last_event.retain(|path, last| {
            if now.duration_since(*last) >= Duration::from_millis(self.debounce_ms) {
                changed.push(path.clone());
                false
            } else {
                true
            }
        });
        changed
    }

    /// Get current offset for a file (or 0 if not tracked).
    pub fn get_offset(&self, path: &Path) -> u64 {
        self.offsets.get(path).copied().unwrap_or(0)
    }

    /// Update offset for a file after reading.
    pub fn set_offset(&mut self, path: PathBuf, offset: u64) {
        self.offsets.insert(path, offset);
    }

    /// Carry forward append offsets across watcher rebuilds so reload does not
    /// re-validate historical file contents.
    pub fn restore_offsets(&mut self, previous: &HashMap<PathBuf, u64>) {
        for (path, offset) in previous {
            if path.exists() {
                // Keep the old offset so ingestion can detect truncation after reload.
                self.offsets.insert(path.clone(), *offset);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FileWatcher;
    use std::collections::HashMap;
    use std::path::PathBuf;

    #[test]
    fn restore_offsets_preserves_truncation_signal() {
        let dir = std::env::temp_dir().join(format!("observer-daemon-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let file = dir.join("session.jsonl");
        std::fs::write(&file, "abc").expect("temp file");

        let mut watcher = FileWatcher::new(&[], 250).expect("watcher");
        let mut previous = HashMap::<PathBuf, u64>::new();
        previous.insert(file.clone(), 999);

        watcher.restore_offsets(&previous);

        assert_eq!(watcher.get_offset(&file), 999);

        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_dir(&dir);
    }
    #[test]
    fn debounce_retains_final_event_until_quiet() {
        let mut watcher = FileWatcher::new(&[], 500).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        watcher.rx = rx;
        let path = PathBuf::from("fixture.jsonl");
        tx.send(Ok(notify::Event::new(notify::EventKind::Any)))
            .unwrap();
        tx.send(Ok(notify::Event::new(notify::EventKind::Create(
            notify::event::CreateKind::File,
        ))
        .add_path(path.clone())))
            .unwrap();
        assert!(watcher.poll().is_empty());
        assert!(watcher.last_event.contains_key(&path));
        watcher.last_event.insert(
            path.clone(),
            std::time::Instant::now() - std::time::Duration::from_secs(1),
        );
        assert_eq!(watcher.poll(), vec![path]);
        assert!(watcher.poll().is_empty());
    }
}
