//! In-memory transfer queue for Sync Settings UI (active + recent done).

use std::collections::{HashMap, VecDeque};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

const MAX_DONE: usize = 100;
/// Cap on how many active+waiting entries `snapshot()` serializes to the UI.
/// The queue itself is unbounded (see the removal of the old `MAX_WAITING`
/// cap below) — this only limits the IPC payload. The Sync panel renders a
/// short list preview plus a count, so truncating the list here loses
/// nothing visible while keeping a 100k-file queue from pushing a
/// multi-megabyte JSON blob through the bridge on every poll.
const SNAPSHOT_ITEMS_CAP: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferDirection {
    Upload,
    Download,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferStatus {
    Active,
    Waiting,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferItem {
    pub id: String,
    pub binding_id: String,
    pub direction: TransferDirection,
    /// Parent path (directory), no trailing slash.
    pub path: String,
    pub name: String,
    pub size: Option<i64>,
    pub status: TransferStatus,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferQueueSnapshot {
    /// Unfinished uploads (active + waiting).
    pub uploading: usize,
    /// Unfinished downloads (active + waiting).
    pub downloading: usize,
    pub items: Vec<TransferItem>,
}

/// Upload totals for a progress display (the Android notification).
///
/// `completed*` count every upload this process has finished, so a display
/// that wants "this run" subtracts the values it saw when the run began.
/// `remaining*` cover what is still active or waiting, sent bytes included.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct UploadProgressSnapshot {
    pub completed: u64,
    pub completed_bytes: u64,
    pub remaining: usize,
    pub remaining_bytes: u64,
    /// Active uploads whose bytes are on the way, oldest first.
    pub current: Vec<UploadInFlight>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UploadInFlight {
    pub name: String,
    pub sent: u64,
    pub size: u64,
}

/// Identifies a waiting slot regardless of its transfer id: same file, same
/// direction, same binding. Used to detect "this path is already queued" and
/// to replace that entry in place instead of scanning `waiting` for it.
type WaitingKey = (String, TransferDirection, String, String);

#[derive(Debug, Default)]
pub struct TransferQueue {
    active: Vec<TransferItem>,
    waiting: Vec<TransferItem>,
    // Position indexes into `waiting`, kept in sync by every method that
    // mutates it. Without these, `enqueue_waiting` and `promote` were doing
    // a linear scan of `waiting` per call, which made enqueuing N files
    // O(N^2) — fine at the old 2000-item cap, not at 100k+.
    waiting_pos_by_id: HashMap<String, usize>,
    waiting_pos_by_key: HashMap<WaitingKey, usize>,
    done: VecDeque<TransferItem>,
    /// Bytes sent so far, per active upload that has started sending.
    meters: HashMap<String, Arc<AtomicU64>>,
    uploads_completed: u64,
    upload_bytes_completed: u64,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn split_path(relative: &str) -> (String, String) {
    let relative = relative.trim_matches('/');
    match relative.rsplit_once('/') {
        Some((p, n)) => (p.to_owned(), n.to_owned()),
        None => (String::new(), relative.to_owned()),
    }
}

impl TransferQueue {
    fn key_of(item: &TransferItem) -> WaitingKey {
        (
            item.binding_id.clone(),
            item.direction,
            item.path.clone(),
            item.name.clone(),
        )
    }

    /// Removes the waiting item at `pos`, keeping both position indexes
    /// consistent. Uses `swap_remove` (O(1)) instead of `remove` (O(n)) —
    /// the moved-in item's index entries are patched to point at `pos`.
    fn waiting_remove_at(&mut self, pos: usize) -> TransferItem {
        let removed = self.waiting.swap_remove(pos);
        self.waiting_pos_by_id.remove(&removed.id);
        self.waiting_pos_by_key.remove(&Self::key_of(&removed));
        if let Some(moved) = self.waiting.get(pos) {
            self.waiting_pos_by_id.insert(moved.id.clone(), pos);
            self.waiting_pos_by_key.insert(Self::key_of(moved), pos);
        }
        removed
    }

    fn waiting_remove_by_key(&mut self, key: &WaitingKey) -> Option<TransferItem> {
        let pos = *self.waiting_pos_by_key.get(key)?;
        Some(self.waiting_remove_at(pos))
    }

    fn waiting_remove_by_id(&mut self, id: &str) -> Option<TransferItem> {
        let pos = *self.waiting_pos_by_id.get(id)?;
        Some(self.waiting_remove_at(pos))
    }

    fn waiting_push(&mut self, item: TransferItem) {
        let pos = self.waiting.len();
        self.waiting_pos_by_id.insert(item.id.clone(), pos);
        self.waiting_pos_by_key.insert(Self::key_of(&item), pos);
        self.waiting.push(item);
    }

    pub fn begin(
        &mut self,
        binding_id: &str,
        direction: TransferDirection,
        relative_path: &str,
        size: Option<i64>,
    ) -> String {
        let (path, name) = split_path(relative_path);
        // Replace any prior active entry for the same binding+direction+path.
        self.active.retain(|i| {
            !(i.binding_id == binding_id
                && i.direction == direction
                && i.path == path
                && i.name == name)
        });
        self.waiting_remove_by_key(&(binding_id.to_owned(), direction, path.clone(), name.clone()));
        let id = Uuid::new_v4().to_string();
        self.active.push(TransferItem {
            id: id.clone(),
            binding_id: binding_id.to_owned(),
            direction,
            path,
            name,
            size,
            status: TransferStatus::Active,
            updated_at_ms: now_ms(),
        });
        id
    }

    /// Enqueues a file as Waiting. The queue is unbounded — there used to be
    /// a 2000-item cap here (`MAX_WAITING`) that made a file silently vanish
    /// from the Sync UI count once a folder had more pending uploads than
    /// that; large first-time syncs hit it constantly. The IPC payload is
    /// capped instead, in `snapshot()`.
    pub fn enqueue_waiting(
        &mut self,
        binding_id: &str,
        direction: TransferDirection,
        relative_path: &str,
        size: Option<i64>,
    ) -> String {
        let (path, name) = split_path(relative_path);
        // Replace any prior active entry for the same binding+direction+path.
        self.active.retain(|i| {
            !(i.binding_id == binding_id
                && i.direction == direction
                && i.path == path
                && i.name == name)
        });
        let key = (binding_id.to_owned(), direction, path.clone(), name.clone());
        self.waiting_remove_by_key(&key);
        let id = Uuid::new_v4().to_string();
        self.waiting_push(TransferItem {
            id: id.clone(),
            binding_id: binding_id.to_owned(),
            direction,
            path,
            name,
            size,
            status: TransferStatus::Waiting,
            updated_at_ms: now_ms(),
        });
        id
    }

    pub fn promote(&mut self, id: &str) -> bool {
        let Some(mut item) = self.waiting_remove_by_id(id) else {
            return false;
        };
        item.status = TransferStatus::Active;
        item.updated_at_ms = now_ms();
        self.active.push(item);
        true
    }

    pub fn complete(&mut self, id: &str) {
        self.meters.remove(id);
        if let Some(mut item) = self.take_active_or_waiting(id) {
            if item.direction == TransferDirection::Upload {
                self.uploads_completed += 1;
                self.upload_bytes_completed += item.size.unwrap_or(0).max(0) as u64;
            }
            item.status = TransferStatus::Done;
            item.updated_at_ms = now_ms();
            self.push_done(item);
        }
    }

    pub fn abandon(&mut self, id: &str) {
        self.meters.remove(id);
        self.active.retain(|i| i.id != id);
        self.waiting_remove_by_id(id);
    }

    /// Drop unfinished items for a binding (e.g. disabled / removed). Rare
    /// and already O(n) via `retain`, so the position indexes are simply
    /// rebuilt afterwards rather than patched entry-by-entry.
    pub fn clear_binding(&mut self, binding_id: &str) {
        self.active.retain(|i| i.binding_id != binding_id);
        self.waiting.retain(|i| i.binding_id != binding_id);
        let active = &self.active;
        self.meters
            .retain(|id, _| active.iter().any(|i| &i.id == id));
        self.waiting_pos_by_id.clear();
        self.waiting_pos_by_key.clear();
        for (pos, item) in self.waiting.iter().enumerate() {
            self.waiting_pos_by_id.insert(item.id.clone(), pos);
            self.waiting_pos_by_key.insert(Self::key_of(item), pos);
        }
    }

    pub fn snapshot(&self) -> TransferQueueSnapshot {
        // Active items are never Waiting and vice versa, so counting each
        // list once by direction is equivalent to (and cheaper than) the old
        // "build the full items vec, then filter it by status".
        let uploading = self
            .active
            .iter()
            .chain(self.waiting.iter())
            .filter(|i| i.direction == TransferDirection::Upload)
            .count();
        let downloading = self
            .active
            .iter()
            .chain(self.waiting.iter())
            .filter(|i| i.direction == TransferDirection::Download)
            .count();

        // The UI only renders a short list preview alongside the counts
        // above, so the serialized list is capped independently of queue
        // size — see `SNAPSHOT_ITEMS_CAP`.
        let mut items = Vec::with_capacity(
            SNAPSHOT_ITEMS_CAP.min(self.active.len() + self.waiting.len()) + self.done.len(),
        );
        items.extend(self.active.iter().take(SNAPSHOT_ITEMS_CAP).cloned());
        let remaining = SNAPSHOT_ITEMS_CAP.saturating_sub(items.len());
        items.extend(self.waiting.iter().take(remaining).cloned());
        items.extend(self.done.iter().cloned());

        TransferQueueSnapshot {
            uploading,
            downloading,
            items,
        }
    }

    /// The byte counter for active transfer `id`, created on first use. The
    /// uploader adds to it as the body goes out.
    pub fn meter(&mut self, id: &str) -> Arc<AtomicU64> {
        self.meters.entry(id.to_owned()).or_default().clone()
    }

    pub fn upload_progress(&self) -> UploadProgressSnapshot {
        let uploads = || {
            self.active
                .iter()
                .chain(self.waiting.iter())
                .filter(|i| i.direction == TransferDirection::Upload)
        };
        let current = self
            .active
            .iter()
            .filter(|i| i.direction == TransferDirection::Upload)
            .filter_map(|i| {
                let sent = self.meters.get(&i.id)?.load(Ordering::Relaxed);
                Some(UploadInFlight {
                    name: i.name.clone(),
                    sent,
                    size: i.size.unwrap_or(0).max(0) as u64,
                })
            })
            .collect();
        UploadProgressSnapshot {
            completed: self.uploads_completed,
            completed_bytes: self.upload_bytes_completed,
            remaining: uploads().count(),
            remaining_bytes: uploads().map(|i| i.size.unwrap_or(0).max(0) as u64).sum(),
            current,
        }
    }

    fn take_active_or_waiting(&mut self, id: &str) -> Option<TransferItem> {
        if let Some(pos) = self.active.iter().position(|i| i.id == id) {
            return Some(self.active.remove(pos));
        }
        self.waiting_remove_by_id(id)
    }

    fn push_done(&mut self, item: TransferItem) {
        self.done.push_front(item);
        while self.done.len() > MAX_DONE {
            self.done.pop_back();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn begin_complete_updates_counts() {
        let mut q = TransferQueue::default();
        let id = q.begin("b1", TransferDirection::Upload, "Camera/a.jpg", Some(10));
        let snap = q.snapshot();
        assert_eq!(snap.uploading, 1);
        assert_eq!(snap.downloading, 0);
        assert_eq!(snap.items[0].status, TransferStatus::Active);
        assert_eq!(snap.items[0].name, "a.jpg");
        assert_eq!(snap.items[0].path, "Camera");

        q.complete(&id);
        let snap = q.snapshot();
        assert_eq!(snap.uploading, 0);
        assert_eq!(snap.items[0].status, TransferStatus::Done);
    }

    #[test]
    fn done_list_is_capped() {
        let mut q = TransferQueue::default();
        for i in 0..(MAX_DONE + 5) {
            let id = q.begin("b", TransferDirection::Download, &format!("f{i}.bin"), None);
            q.complete(&id);
        }
        assert_eq!(q.snapshot().items.len(), MAX_DONE);
    }

    #[test]
    fn enqueue_waiting_then_promote_then_complete() {
        let mut q = TransferQueue::default();
        let id = q.enqueue_waiting("b1", TransferDirection::Upload, "Camera/a.jpg", Some(10));
        let snap = q.snapshot();
        assert_eq!(snap.uploading, 1);
        assert_eq!(snap.items[0].status, TransferStatus::Waiting);
        assert!(q.promote(&id));
        assert_eq!(q.snapshot().items[0].status, TransferStatus::Active);
        q.complete(&id);
        assert_eq!(q.snapshot().uploading, 0);
        assert_eq!(q.snapshot().items[0].status, TransferStatus::Done);
    }

    #[test]
    fn upload_progress_counts_finished_queued_and_sent_bytes() {
        let mut q = TransferQueue::default();
        let a = q.enqueue_waiting("b", TransferDirection::Upload, "Camera/a.jpg", Some(100));
        q.enqueue_waiting("b", TransferDirection::Upload, "Camera/b.jpg", Some(50));
        q.begin("b", TransferDirection::Download, "x.bin", Some(7));
        let p = q.upload_progress();
        assert_eq!((p.remaining, p.remaining_bytes), (2, 150));
        assert!(p.current.is_empty(), "nothing is sending yet");

        assert!(q.promote(&a));
        q.meter(&a).fetch_add(40, Ordering::Relaxed);
        let p = q.upload_progress();
        assert_eq!(
            p.current,
            vec![UploadInFlight {
                name: "a.jpg".into(),
                sent: 40,
                size: 100
            }]
        );

        q.complete(&a);
        let p = q.upload_progress();
        assert_eq!((p.completed, p.completed_bytes), (1, 100));
        assert_eq!((p.remaining, p.remaining_bytes), (1, 50));
        assert!(p.current.is_empty());
        assert!(q.meters.is_empty(), "a finished upload drops its meter");
    }

    #[test]
    fn enqueue_waiting_is_unbounded_and_snapshot_items_are_capped() {
        let mut q = TransferQueue::default();
        for i in 0..5000 {
            q.enqueue_waiting("b", TransferDirection::Upload, &format!("f{i}.jpg"), None);
        }
        let snap = q.snapshot();
        assert_eq!(snap.uploading, 5000);
        assert!(snap.items.len() <= SNAPSHOT_ITEMS_CAP);
    }
}
