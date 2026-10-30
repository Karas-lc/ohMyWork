use crate::{
    atomic_write::{
        AtomicWriteError, CommitAck, DiskSnapshot, DiskVersion, atomic_write, read_snapshot,
    },
    identity::DocumentKey,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError},
    },
    thread,
    time::Duration,
};

pub const ORDINARY_QUEUE_CAPACITY: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueError {
    Stopped,
    OrdinaryQueueFull,
}

struct Job {
    key: DocumentKey,
    run: Box<dyn FnOnce() + Send + 'static>,
}

struct Finished {
    key: DocumentKey,
}

struct QueueInner {
    save_tx: Sender<Job>,
    ordinary_tx: SyncSender<Job>,
    stop_tx: Sender<()>,
    stopped: AtomicBool,
}

pub struct WriteQueue {
    inner: Arc<QueueInner>,
}

impl WriteQueue {
    pub fn new(worker_count: usize) -> Self {
        let worker_count = worker_count.max(1);
        let (save_tx, save_rx) = mpsc::channel::<Job>();
        let (ordinary_tx, ordinary_rx) = mpsc::sync_channel::<Job>(ORDINARY_QUEUE_CAPACITY);
        let (finished_tx, finished_rx) = mpsc::channel::<Finished>();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let mut worker_txs = Vec::with_capacity(worker_count);

        for index in 0..worker_count {
            let (worker_tx, worker_rx) = mpsc::channel::<Job>();
            let finished_tx = finished_tx.clone();
            let name = format!("omw-local-write-{index}");
            let _ = thread::Builder::new().name(name).spawn(move || {
                while let Ok(job) = worker_rx.recv() {
                    (job.run)();
                    if finished_tx.send(Finished { key: job.key }).is_err() {
                        break;
                    }
                }
            });
            worker_txs.push(worker_tx);
        }
        drop(finished_tx);

        let _ = thread::Builder::new()
            .name("omw-local-write-dispatch".into())
            .spawn(move || dispatch_loop(save_rx, ordinary_rx, finished_rx, stop_rx, worker_txs));

        Self {
            inner: Arc::new(QueueInner {
                save_tx,
                ordinary_tx,
                stop_tx,
                stopped: AtomicBool::new(false),
            }),
        }
    }

    pub fn submit_save(
        &self,
        key: DocumentKey,
        path: impl Into<PathBuf>,
        expected: Option<DiskVersion>,
        content: Vec<u8>,
        request_revision: u64,
    ) -> Result<Receiver<Result<CommitAck, AtomicWriteError>>, QueueError> {
        self.ensure_running()?;
        let (result_tx, result_rx) = mpsc::channel();
        let path = path.into();
        let job = Job {
            key,
            run: Box::new(move || {
                let result = atomic_write(&path, expected.as_ref(), &content, request_revision);
                let _ = result_tx.send(result);
            }),
        };
        self.inner.save_tx.send(job).map_err(|_| {
            self.inner.stopped.store(true, Ordering::Release);
            QueueError::Stopped
        })?;
        Ok(result_rx)
    }
    /// Task 局部提交请求（计划14）。
    /// 调用方先用 editor 的 Task 合并逻辑算最小 B' 后提交。
    /// expected 传基线版本。
    pub fn submit_task_save(
        &self,
        key: DocumentKey,
        path: impl Into<PathBuf>,
        expected: Option<DiskVersion>,
        patched_bytes: Vec<u8>,
        request_revision: u64,
    ) -> Result<Receiver<Result<CommitAck, AtomicWriteError>>, QueueError> {
        self.submit_save(key, path, expected, patched_bytes, request_revision)
    }


    pub fn submit_read(
        &self,
        key: DocumentKey,
        path: impl Into<PathBuf>,
    ) -> Result<Receiver<Result<DiskSnapshot, AtomicWriteError>>, QueueError> {
        self.ensure_running()?;
        let (result_tx, result_rx) = mpsc::channel();
        let path = path.into();
        let job = Job {
            key,
            run: Box::new(move || {
                let _ = result_tx.send(read_snapshot(path));
            }),
        };
        self.inner
            .ordinary_tx
            .try_send(job)
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => QueueError::OrdinaryQueueFull,
                mpsc::TrySendError::Disconnected(_) => {
                    self.inner.stopped.store(true, Ordering::Release);
                    QueueError::Stopped
                }
            })?;
        Ok(result_rx)
    }

    pub fn shutdown(&self) {
        if !self.inner.stopped.swap(true, Ordering::AcqRel) {
            let _ = self.inner.stop_tx.send(());
        }
    }

    fn ensure_running(&self) -> Result<(), QueueError> {
        if self.inner.stopped.load(Ordering::Acquire) {
            Err(QueueError::Stopped)
        } else {
            Ok(())
        }
    }
}

impl Default for WriteQueue {
    fn default() -> Self {
        Self::new(2)
    }
}

impl Clone for WriteQueue {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl Drop for WriteQueue {
    fn drop(&mut self) {
        if Arc::strong_count(&self.inner) == 1 {
            self.shutdown();
        }
    }
}

fn dispatch_loop(
    save_rx: Receiver<Job>,
    ordinary_rx: Receiver<Job>,
    finished_rx: Receiver<Finished>,
    stop_rx: Receiver<()>,
    worker_txs: Vec<Sender<Job>>,
) {
    let worker_count = worker_txs.len();
    let mut pending: HashMap<DocumentKey, VecDeque<Job>> = HashMap::new();
    let mut active = HashSet::new();
    let mut next_worker = 0usize;

    loop {
        let mut changed = false;
        loop {
            match save_rx.try_recv() {
                Ok(job) => {
                    pending.entry(job.key.clone()).or_default().push_back(job);
                    changed = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        loop {
            match ordinary_rx.try_recv() {
                Ok(job) => {
                    pending.entry(job.key.clone()).or_default().push_back(job);
                    changed = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        loop {
            match finished_rx.try_recv() {
                Ok(done) => {
                    active.remove(&done.key);
                    changed = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }

        while active.len() < worker_count {
            let Some(key) = pending.keys().find(|key| !active.contains(*key)).cloned() else {
                break;
            };
            let Some(queue) = pending.get_mut(&key) else {
                continue;
            };
            let Some(job) = queue.pop_front() else {
                pending.remove(&key);
                continue;
            };
            if queue.is_empty() {
                pending.remove(&key);
            }
            let worker = next_worker % worker_count;
            next_worker = next_worker.wrapping_add(1);
            if worker_txs[worker].send(job).is_err() {
                return;
            }
            active.insert(key);
            changed = true;
        }

        if !changed {
            match stop_rx.try_recv() {
                Ok(()) | Err(TryRecvError::Disconnected) => return,
                Err(TryRecvError::Empty) => {}
            }
            match save_rx.recv_timeout(Duration::from_millis(10)) {
                Ok(job) => {
                    pending.entry(job.key.clone()).or_default().push_back(job);
                }
                Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }
}
