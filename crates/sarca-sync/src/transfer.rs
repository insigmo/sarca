//! In-memory transfer queue for Sync Settings UI (active + recent done).

use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

const MAX_DONE: usize = 100;
pub const MAX_WAITING: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Default)]
pub struct TransferQueue {
    active: Vec<TransferItem>,
    waiting: Vec<TransferItem>,
    done: VecDeque<TransferItem>,
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
        self.waiting.retain(|i| {
            !(i.binding_id == binding_id
                && i.direction == direction
                && i.path == path
                && i.name == name)
        });
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

    pub fn enqueue_waiting(
        &mut self,
        binding_id: &str,
        direction: TransferDirection,
        relative_path: &str,
        size: Option<i64>,
    ) -> Option<String> {
        let (path, name) = split_path(relative_path);
        let matches = |i: &TransferItem| {
            i.binding_id == binding_id
                && i.direction == direction
                && i.path == path
                && i.name == name
        };
        // Check the cap *before* evicting any matching Active entry: if we're
        // over cap and there's no existing Waiting row to replace in-place,
        // bail out without touching Active — otherwise an in-flight upload
        // would be silently dropped from the queue for nothing (it stays
        // Active, just not re-enqueued as Waiting).
        let has_existing_waiting = self.waiting.iter().any(matches);
        if !has_existing_waiting && self.waiting.len() >= MAX_WAITING {
            return None;
        }
        self.active.retain(|i| !matches(i));
        self.waiting.retain(|i| !matches(i));
        let id = Uuid::new_v4().to_string();
        self.waiting.push(TransferItem {
            id: id.clone(),
            binding_id: binding_id.to_owned(),
            direction,
            path,
            name,
            size,
            status: TransferStatus::Waiting,
            updated_at_ms: now_ms(),
        });
        Some(id)
    }

    pub fn promote(&mut self, id: &str) -> bool {
        let Some(pos) = self.waiting.iter().position(|i| i.id == id) else {
            return false;
        };
        let mut item = self.waiting.remove(pos);
        item.status = TransferStatus::Active;
        item.updated_at_ms = now_ms();
        self.active.push(item);
        true
    }

    pub fn complete(&mut self, id: &str) {
        if let Some(mut item) = self.take_active_or_waiting(id) {
            item.status = TransferStatus::Done;
            item.updated_at_ms = now_ms();
            self.push_done(item);
        }
    }

    pub fn abandon(&mut self, id: &str) {
        self.active.retain(|i| i.id != id);
        self.waiting.retain(|i| i.id != id);
    }

    /// Drop unfinished items for a binding (e.g. disabled / removed).
    pub fn clear_binding(&mut self, binding_id: &str) {
        self.active.retain(|i| i.binding_id != binding_id);
        self.waiting.retain(|i| i.binding_id != binding_id);
    }

    pub fn snapshot(&self) -> TransferQueueSnapshot {
        let mut items =
            Vec::with_capacity(self.active.len() + self.waiting.len() + self.done.len());
        items.extend(self.active.iter().cloned());
        items.extend(self.waiting.iter().cloned());
        items.extend(self.done.iter().cloned());
        let uploading = items
            .iter()
            .filter(|i| {
                i.direction == TransferDirection::Upload
                    && matches!(i.status, TransferStatus::Active | TransferStatus::Waiting)
            })
            .count();
        let downloading = items
            .iter()
            .filter(|i| {
                i.direction == TransferDirection::Download
                    && matches!(i.status, TransferStatus::Active | TransferStatus::Waiting)
            })
            .count();
        TransferQueueSnapshot {
            uploading,
            downloading,
            items,
        }
    }

    fn take_active_or_waiting(&mut self, id: &str) -> Option<TransferItem> {
        if let Some(pos) = self.active.iter().position(|i| i.id == id) {
            return Some(self.active.remove(pos));
        }
        if let Some(pos) = self.waiting.iter().position(|i| i.id == id) {
            return Some(self.waiting.remove(pos));
        }
        None
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
        let id = q
            .enqueue_waiting("b1", TransferDirection::Upload, "Camera/a.jpg", Some(10))
            .expect("enqueued");
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
    fn waiting_cap_returns_none_beyond_limit() {
        let mut q = TransferQueue::default();
        for i in 0..MAX_WAITING {
            assert!(q
                .enqueue_waiting("b", TransferDirection::Upload, &format!("f{i}.jpg"), None)
                .is_some());
        }
        assert!(q
            .enqueue_waiting("b", TransferDirection::Upload, "overflow.jpg", None)
            .is_none());
        assert_eq!(q.snapshot().uploading, MAX_WAITING);
    }
}
