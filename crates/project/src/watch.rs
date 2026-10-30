use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    path::Path,
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
};

use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::{
    AtomicWriteError, CommitAck, DiskSnapshot, DiskVersion,
    identity::{DocumentKey, is_application_temporary},
    read_snapshot,
};

pub const WATCH_EVENT_QUEUE_CAPACITY: usize = 256;
const RECENT_COMMIT_CAPACITY: usize = 8;

type QueueItem = Result<DocumentKey, String>;

#[derive(Clone, Debug)]
pub struct WatchEventSender {
    tx: SyncSender<QueueItem>,
}

impl WatchEventSender {
    pub fn send(&self, key: DocumentKey) -> Result<(), WatchQueueError> {
        self.tx.send(Ok(key)).map_err(|_| WatchQueueError::Closed)
    }

    fn send_error(&self, error: String) -> Result<(), WatchQueueError> {
        self.tx
            .send(Err(error))
            .map_err(|_| WatchQueueError::Closed)
    }
}

pub struct WatchEventQueue {
    sender: WatchEventSender,
    receiver: Receiver<QueueItem>,
}

impl WatchEventQueue {
    pub fn new(capacity: usize) -> Self {
        let (tx, receiver) = mpsc::sync_channel(capacity.max(1));
        Self {
            sender: WatchEventSender { tx },
            receiver,
        }
    }

    pub fn sender(&self) -> WatchEventSender {
        self.sender.clone()
    }

    pub fn enqueue(&self, key: DocumentKey) -> Result<(), WatchQueueError> {
        self.sender.send(key)
    }

    pub fn drain(&self) -> WatchBatch {
        let mut paths = BTreeSet::new();
        let mut errors = Vec::new();
        loop {
            match self.receiver.try_recv() {
                Ok(Ok(key)) => {
                    paths.insert(key);
                }
                Ok(Err(error)) => errors.push(error),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        WatchBatch {
            paths: paths.into_iter().collect(),
            errors,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WatchQueueError {
    Closed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WatchBatch {
    pub paths: Vec<DocumentKey>,
    pub errors: Vec<String>,
}

/// notify only reports that a path may have changed. The caller must verify
/// the path with [`ChangeTracker::check`] before touching a document session.
pub struct FileWatcher {
    _watcher: RecommendedWatcher,
    queue: WatchEventQueue,
}

impl FileWatcher {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, notify::Error> {
        let queue = WatchEventQueue::new(WATCH_EVENT_QUEUE_CAPACITY);
        let sender = queue.sender();
        let mut watcher = RecommendedWatcher::new(
            move |result: notify::Result<Event>| match result {
                Ok(event) => enqueue_event(&sender, event),
                Err(error) => {
                    let _ = sender.send_error(error.to_string());
                }
            },
            Config::default(),
        )?;
        watcher.watch(root.as_ref(), RecursiveMode::Recursive)?;
        Ok(Self {
            _watcher: watcher,
            queue,
        })
    }

    pub fn poll(&self) -> WatchBatch {
        self.queue.drain()
    }

    pub fn queue(&self) -> &WatchEventQueue {
        &self.queue
    }
}

fn enqueue_event(sender: &WatchEventSender, event: Event) {
    for path in event.paths {
        if is_application_temporary(&path) {
            continue;
        }
        let _ = sender.send(DocumentKey::new(path));
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WatchChange {
    SameContent {
        snapshot: DiskSnapshot,
    },
    SelfCommit {
        snapshot: DiskSnapshot,
        request_revision: u64,
    },
    External {
        snapshot: DiskSnapshot,
    },
    Deleted,
}

#[derive(Clone, Debug)]
struct CommitMarker {
    request_revision: u64,
    version: DiskVersion,
}

/// Re-reads a notified path and classifies it from content plus known commit
/// identity. It intentionally has no time-based suppression rule.
#[derive(Default)]
pub struct ChangeTracker {
    confirmed: HashMap<DocumentKey, DiskVersion>,
    commits: HashMap<DocumentKey, VecDeque<CommitMarker>>,
}

impl ChangeTracker {
    pub fn register(&mut self, key: DocumentKey, version: DiskVersion) {
        self.confirmed.insert(key, version);
    }

    pub fn record_commit(&mut self, ack: &CommitAck) {
        let key = DocumentKey::new(ack.path.clone());
        let markers = self.commits.entry(key.clone()).or_default();
        markers.push_back(CommitMarker {
            request_revision: ack.request_revision,
            version: ack.new_version.clone(),
        });
        while markers.len() > RECENT_COMMIT_CAPACITY {
            markers.pop_front();
        }
        self.confirmed.insert(key, ack.new_version.clone());
    }

    pub fn check(
        &mut self,
        key: &DocumentKey,
        path: impl AsRef<Path>,
    ) -> Result<WatchChange, AtomicWriteError> {
        let path = path.as_ref();
        let snapshot = match read_snapshot(path) {
            Ok(snapshot) => snapshot,
            Err(AtomicWriteError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                self.confirmed.remove(key);
                self.commits.remove(key);
                return Ok(WatchChange::Deleted);
            }
            Err(error) => return Err(error),
        };

        if let Some(marker) = self.commits.get(key).and_then(|markers| {
            markers
                .iter()
                .rev()
                .find(|marker| marker.version.same_content(&snapshot.version))
        }) {
            self.confirmed.insert(key.clone(), snapshot.version.clone());
            return Ok(WatchChange::SelfCommit {
                snapshot,
                request_revision: marker.request_revision,
            });
        }

        let change = if self
            .confirmed
            .get(key)
            .is_some_and(|version| version.same_content(&snapshot.version))
        {
            WatchChange::SameContent {
                snapshot: snapshot.clone(),
            }
        } else {
            WatchChange::External {
                snapshot: snapshot.clone(),
            }
        };
        self.confirmed.insert(key.clone(), snapshot.version);
        Ok(change)
    }

    pub fn forget(&mut self, key: &DocumentKey) {
        self.confirmed.remove(key);
        self.commits.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};
    use uuid::Uuid;

    fn temp_file(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("ohmywork-watch-{name}-{}", Uuid::new_v4()));
        fs::create_dir(&root).expect("create directory");
        let path = root.join("note.md");
        fs::write(&path, b"baseline").expect("write baseline");
        path
    }

    #[test]
    fn bounded_queue_backpressures_without_dropping_paths() {
        let queue = WatchEventQueue::new(1);
        let sender = queue.sender();
        sender
            .send(DocumentKey::new("first.md"))
            .expect("first event");
        let blocked = std::thread::spawn(move || {
            sender
                .send(DocumentKey::new("second.md"))
                .expect("second event");
        });
        let first = queue.drain();
        blocked.join().expect("backpressure sender");
        let second = queue.drain();
        let paths = first
            .paths
            .into_iter()
            .chain(second.paths)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn duplicate_and_reordered_events_are_merged_but_external_content_is_not() {
        let path = temp_file("classification");
        let key = DocumentKey::new(&path);
        let initial = crate::read_snapshot(&path).expect("initial snapshot");
        let mut tracker = ChangeTracker::default();
        tracker.register(key.clone(), initial.version);

        let queue = WatchEventQueue::new(8);
        queue.enqueue(key.clone()).expect("event");
        queue.enqueue(key.clone()).expect("duplicate event");
        assert_eq!(queue.drain().paths, vec![key.clone()]);

        fs::write(&path, b"external").expect("external edit");
        assert!(matches!(
            tracker.check(&key, &path).expect("check external"),
            WatchChange::External { .. }
        ));
        fs::write(&path, b"external").expect("metadata event");
        assert!(matches!(
            tracker.check(&key, &path).expect("check duplicate"),
            WatchChange::SameContent { .. }
        ));
        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn commit_ack_identifies_self_write_without_time_window() {
        let path = temp_file("commit");
        let key = DocumentKey::new(&path);
        let initial = crate::read_snapshot(&path).expect("initial snapshot");
        let ack =
            crate::atomic_write(&path, Some(&initial.version), b"committed", 41).expect("commit");
        let mut tracker = ChangeTracker::default();
        tracker.register(key.clone(), initial.version);
        tracker.record_commit(&ack);
        assert!(matches!(
            tracker.check(&key, &path).expect("check commit"),
            WatchChange::SelfCommit {
                request_revision: 41,
                ..
            }
        ));
        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn delete_and_recreate_are_distinct_changes() {
        let path = temp_file("recreate");
        let key = DocumentKey::new(&path);
        let initial = crate::read_snapshot(&path).expect("initial snapshot");
        let mut tracker = ChangeTracker::default();
        tracker.register(key.clone(), initial.version);
        fs::remove_file(&path).expect("delete file");
        assert_eq!(
            tracker.check(&key, &path).expect("check delete"),
            WatchChange::Deleted
        );
        fs::write(&path, b"recreated").expect("recreate file");
        assert!(matches!(
            tracker.check(&key, &path).expect("check recreate"),
            WatchChange::External { .. }
        ));
        let _ = fs::remove_dir_all(path.parent().expect("parent"));
    }
}
