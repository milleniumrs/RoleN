//! Global write queue (PRD FR-7.3/7.4/7.6/7.7).
//!
//! Architecture: one dispatcher thread owns two maps — `inbox` (path → FIFO
//! backlog) and `inflight` (paths currently being applied). A ticket for an
//! idle path is dispatched to a short-lived applier thread immediately; a
//! ticket for a busy path waits in its FIFO. Appliers report completion back
//! through the same channel, the dispatcher then pops the path's next ticket.
//! Result: strict per-path ordering, real concurrency across disjoint paths.

use rolen_core::ledger::Ledger;
use rolen_core::types::{TicketState, WriteOp, WriteTicket};
use rolen_runtime::error::RuntimeError;
use rolen_runtime::sink::{resolve_in, WriteSink};
use std::collections::{HashMap, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

enum Msg {
    New(WriteTicket, Sender<TicketState>),
    Done {
        path: PathBuf,
        state: TicketState,
        respond: Sender<TicketState>,
    },
    Shutdown,
}

pub struct WriteQueue {
    tx: Sender<Msg>,
    depth: Arc<AtomicUsize>,
    dispatcher: Option<std::thread::JoinHandle<()>>,
}

impl WriteQueue {
    /// `root` is the workspace every ticket path is sandboxed to.
    pub fn new(root: PathBuf) -> Arc<Self> {
        let (tx, rx) = channel::<Msg>();
        let depth = Arc::new(AtomicUsize::new(0));
        let worker_tx = tx.clone();
        let dispatcher = std::thread::spawn(move || dispatcher_loop(root, rx, worker_tx));
        Arc::new(Self {
            tx,
            depth,
            dispatcher: Some(dispatcher),
        })
    }

    /// Submit a ticket; the returned handle waits for the verdict.
    pub fn submit(&self, mut ticket: WriteTicket) -> TicketHandle {
        ticket.state = TicketState::Queued;
        self.depth.fetch_add(1, Ordering::Relaxed);
        let (rtx, rrx) = channel();
        // journal the submission (best-effort; queue must not fail on db errors)
        if let Ok(ledger) = Ledger::open_default() {
            let _ = ledger.insert_ticket(&ticket);
        }
        let id = ticket.id.clone();
        let _ = self.tx.send(Msg::New(ticket, rtx));
        TicketHandle {
            id,
            rx: rrx,
            depth: self.depth.clone(),
        }
    }

    /// Tickets currently waiting or being applied.
    pub fn depth(&self) -> usize {
        self.depth.load(Ordering::Relaxed)
    }

    pub fn shutdown(&self) {
        let _ = self.tx.send(Msg::Shutdown);
    }
}

impl Drop for WriteQueue {
    fn drop(&mut self) {
        self.shutdown();
        if let Some(h) = self.dispatcher.take() {
            let _ = h.join();
        }
    }
}

pub struct TicketHandle {
    pub id: String,
    rx: Receiver<TicketState>,
    depth: Arc<AtomicUsize>,
}

impl TicketHandle {
    pub fn wait(self) -> TicketState {
        let state = self.rx.recv().unwrap_or(TicketState::Rejected);
        self.depth.fetch_sub(1, Ordering::Relaxed);
        state
    }
}

/// WriteSink facade used by agent threads: submit + wait (FR-7.1).
pub struct QueuedWriteSink {
    queue: Arc<WriteQueue>,
}

impl QueuedWriteSink {
    pub fn new(queue: Arc<WriteQueue>) -> Self {
        Self { queue }
    }
}

impl WriteSink for QueuedWriteSink {
    fn apply(&self, ticket: &WriteTicket) -> Result<TicketState, RuntimeError> {
        let handle = self.queue.submit(ticket.clone());
        Ok(handle.wait())
    }
}

// ------------------------------------------------------------- dispatcher

fn dispatcher_loop(root: PathBuf, rx: Receiver<Msg>, worker_tx: Sender<Msg>) {
    let mut inbox: HashMap<PathBuf, VecDeque<(WriteTicket, Sender<TicketState>)>> = HashMap::new();
    let mut inflight: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    loop {
        match rx.recv() {
            Ok(Msg::New(ticket, rtx)) => {
                let path = ticket.path.clone();
                if inflight.contains(&path) {
                    inbox.entry(path).or_default().push_back((ticket, rtx));
                } else {
                    inflight.insert(path.clone());
                    dispatch(&root, &path, ticket, rtx, &worker_tx);
                }
            }
            Ok(Msg::Done {
                path,
                state,
                respond,
                ..
            }) => {
                let _ = respond.send(state);
                match inbox.get_mut(&path) {
                    Some(q) => {
                        if let Some((next, rtx)) = q.pop_front() {
                            dispatch(&root, &path, next, rtx, &worker_tx);
                            // path stays inflight
                        } else {
                            inbox.remove(&path);
                            inflight.remove(&path);
                        }
                    }
                    None => {
                        inflight.remove(&path);
                    }
                }
            }
            Ok(Msg::Shutdown) | Err(_) => break,
        }
    }
}

fn dispatch(
    root: &Path,
    path: &Path,
    ticket: WriteTicket,
    respond: Sender<TicketState>,
    worker_tx: &Sender<Msg>,
) {
    let root = root.to_path_buf();
    let path = path.to_path_buf();
    let tx = worker_tx.clone();
    std::thread::spawn(move || {
        let state = apply_ticket(&root, &ticket);
        if let Ok(ledger) = Ledger::open_default() {
            let _ = ledger.set_ticket_state(&ticket.id, state);
        }
        let _ = tx.send(Msg::Done {
            path,
            state,
            respond,
        });
    });
}

// ----------------------------------------------------------------- apply

/// Content hash for optimistic concurrency (FR-7.4). Not security-grade —
/// it only detects "file changed since the agent read it".
fn content_hash(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    Some(format!("{:016x}", h.finish()))
}

fn apply_ticket(root: &Path, ticket: &WriteTicket) -> TicketState {
    let path = match resolve_in(root, &ticket.path.to_string_lossy()) {
        Ok(p) => p,
        Err(_) => return TicketState::Rejected,
    };

    // optimistic concurrency: the ticket must name the file version it was
    // based on (None = "file did not exist")
    let current = content_hash(&path);
    let stale = match (&ticket.base_hash, &current) {
        (None, None) => false,
        (Some(want), Some(got)) if want == got => false,
        // no base hash supplied → accept but we are overwriting blind
        (None, Some(_)) => matches!(ticket.op, WriteOp::Create),
        _ => true,
    };
    if stale {
        return TicketState::Rejected;
    }

    let result = match ticket.op {
        WriteOp::Create | WriteOp::Replace => (|| {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let tmp = path.with_extension("rolen-tmp");
            std::fs::write(&tmp, &ticket.payload)?;
            std::fs::rename(&tmp, &path)
        })(),
        WriteOp::Delete => {
            if path.exists() {
                std::fs::remove_file(&path)
            } else {
                Ok(())
            }
        }
        WriteOp::Rename => match resolve_in(root, &ticket.payload) {
            Ok(target) => std::fs::rename(&path, &target),
            Err(_) => return TicketState::Rejected,
        },
        WriteOp::Patch => return TicketState::Rejected, // P1 (FR-7.9)
    };
    match result {
        Ok(()) => TicketState::Applied,
        Err(_) => TicketState::Rejected,
    }
}

/// Public helper: hash a file for agents that want to attach base_hash.
pub fn hash_file(root: &Path, rel: &str) -> Option<String> {
    resolve_in(root, rel).ok().and_then(|p| content_hash(&p))
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn ticket(id: &str, path: &str, payload: &str, base_hash: Option<String>) -> WriteTicket {
        WriteTicket {
            id: id.into(),
            task_id: "test".into(),
            path: path.into(),
            op: WriteOp::Replace,
            payload: payload.into(),
            base_hash,
            state: TicketState::Queued,
            ts: Utc::now(),
        }
    }

    fn testdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rolen-queue-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn hash_reject_on_stale_base() {
        let dir = testdir("stale");
        let q = WriteQueue::new(dir.clone());

        // first writer creates v1 (no base hash: file doesn't exist yet)
        let s = q.submit(ticket("t1", "a.txt", "v1", None)).wait();
        assert_eq!(s, TicketState::Applied);

        // second writer read v1 and bases its write on v1's hash → ok
        let v1_hash = hash_file(&dir, "a.txt").unwrap();
        let s = q
            .submit(ticket("t2", "a.txt", "v2", Some(v1_hash.clone())))
            .wait();
        assert_eq!(s, TicketState::Applied);

        // third writer still holds the OLD v1 hash → stale → rejected
        let s = q.submit(ticket("t3", "a.txt", "v3", Some(v1_hash))).wait();
        assert_eq!(s, TicketState::Rejected);
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "v2");

        q.shutdown();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn per_path_fifo_order() {
        let dir = testdir("fifo");
        let q = WriteQueue::new(dir.clone());
        // 50 tickets against the same path must apply in submission order
        let mut handles = Vec::new();
        for i in 0..50 {
            handles.push(q.submit(ticket(&format!("t{i}"), "seq.txt", &format!("{i}"), None)));
        }
        let mut applied = 0;
        for h in handles {
            if h.wait() == TicketState::Applied {
                applied += 1;
            }
        }
        // all blind replaces accepted; content must be the LAST submitted one
        assert_eq!(applied, 50);
        assert_eq!(std::fs::read_to_string(dir.join("seq.txt")).unwrap(), "49");
        q.shutdown();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn disjoint_paths_all_apply() {
        let dir = testdir("disjoint");
        let q = WriteQueue::new(dir.clone());
        let mut handles = Vec::new();
        for i in 0..10 {
            handles.push(q.submit(ticket(&format!("t{i}"), &format!("f{i}.txt"), "x", None)));
        }
        for h in handles {
            assert_eq!(h.wait(), TicketState::Applied);
        }
        for i in 0..10 {
            assert!(dir.join(format!("f{i}.txt")).exists());
        }
        q.shutdown();
        std::fs::remove_dir_all(&dir).ok();
    }
}
