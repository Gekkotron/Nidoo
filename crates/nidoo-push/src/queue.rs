//! In-memory retry queue, ported from OikosPush's `NotificationQueue` with an injected clock
//! (`now` is epoch seconds). The queue never sends: `take_due` hands items out, the caller
//! puts failures back with `put_back`, and a success is simply not put back.

use std::collections::BTreeMap;

use crate::normalize::Priority;

/// A notification waiting to be sent. `tokens: None` means "resolve at send time".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    pub id: String,
    pub data: BTreeMap<String, String>,
    pub tokens: Option<Vec<String>>,
    pub priority: Priority,
    pub ttl: u32,
    pub queued_at: u64,
    pub attempts: u32,
    pub next_at: u64,
}

#[derive(Debug)]
pub struct RetryQueue {
    retry_interval: u64,
    max_retries: u32,
    items: Vec<Pending>,
}

impl RetryQueue {
    pub fn new(retry_interval: u64, max_retries: u32) -> Self {
        Self {
            retry_interval,
            max_retries,
            items: Vec::new(),
        }
    }

    pub fn add(&mut self, p: Pending) {
        self.items.push(p);
    }

    /// Removes and returns the items ready at `now`. Items past `queued_at + ttl` or at
    /// `max_retries` attempts are dropped; items not yet due stay queued.
    pub fn take_due(&mut self, now: u64) -> Vec<Pending> {
        let (max_retries, mut due) = (self.max_retries, Vec::new());
        self.items.retain_mut(|p| {
            if now > p.queued_at + u64::from(p.ttl) {
                tracing::warn!(id = %p.id, "notification expired (ttl)");
                false
            } else if p.attempts >= max_retries {
                tracing::error!(id = %p.id, "max retries reached");
                false
            } else if now >= p.next_at {
                due.push(p.clone());
                false
            } else {
                true
            }
        });
        due
    }

    /// Re-queues a failed item: one more attempt, next try after the retry interval.
    pub fn put_back(&mut self, mut p: Pending, now: u64) {
        p.attempts += 1;
        p.next_at = now + self.retry_interval;
        tracing::warn!(id = %p.id, attempt = p.attempts, max = self.max_retries, "retry failed");
        self.items.push(p);
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(ttl: u32, now: u64) -> Pending {
        Pending {
            id: "a".into(),
            data: BTreeMap::from([("id".to_string(), "a".to_string())]),
            tokens: Some(vec!["t1".into()]),
            priority: Priority::High,
            ttl,
            queued_at: now,
            attempts: 0,
            next_at: now,
        }
    }

    #[test]
    fn new_item_is_ready_immediately() {
        let mut q = RetryQueue::new(60, 3);
        q.add(pending(43200, 1000));
        let due = q.take_due(1000);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, "a");
        assert_eq!(q.len(), 0, "handed-out items leave the queue");
    }

    #[test]
    fn failure_delays_next_attempt_by_retry_interval() {
        let mut q = RetryQueue::new(60, 3);
        q.add(pending(43200, 1000));
        let item = q.take_due(1000).pop().unwrap();
        q.put_back(item, 1000);
        assert_eq!(q.len(), 1);
        assert!(q.take_due(1000).is_empty());
        assert!(q.take_due(1059).is_empty());
        let due = q.take_due(1061);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].attempts, 1);
        assert_eq!(due[0].next_at, 1060);
    }

    #[test]
    fn item_is_discarded_after_max_retries() {
        let mut q = RetryQueue::new(60, 2);
        q.add(pending(43200, 1000));
        let mut now = 1000;
        for _ in 0..2 {
            let item = q.take_due(now).pop().unwrap();
            q.put_back(item, now);
            now += 61;
        }
        assert!(q.take_due(now).is_empty());
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn item_is_discarded_when_ttl_expires() {
        let mut q = RetryQueue::new(60, 3);
        q.add(pending(100, 1000));
        assert!(q.take_due(1101).is_empty());
        assert_eq!(q.len(), 0);
    }

    #[test]
    fn item_is_still_due_at_exactly_ttl() {
        let mut q = RetryQueue::new(60, 3);
        q.add(pending(100, 1000));
        assert_eq!(q.take_due(1100).len(), 1);
    }

    #[test]
    fn success_is_not_putting_back() {
        let mut q = RetryQueue::new(60, 3);
        q.add(pending(43200, 1000));
        let _sent = q.take_due(1000);
        assert_eq!(q.len(), 0);
        assert!(q.take_due(5000).is_empty());
    }

    #[test]
    fn mutated_tokens_survive_put_back() {
        let mut q = RetryQueue::new(60, 3);
        let mut p = pending(43200, 1000);
        p.tokens = Some(vec!["a".into(), "b".into()]);
        q.add(p);
        let mut item = q.take_due(1000).pop().unwrap();
        item.tokens = Some(vec!["b".into()]);
        q.put_back(item, 1000);
        let due = q.take_due(1060);
        assert_eq!(due[0].tokens, Some(vec!["b".to_string()]));
    }

    #[test]
    fn unresolved_tokens_stay_none() {
        let mut q = RetryQueue::new(60, 3);
        let mut p = pending(43200, 1000);
        p.tokens = None;
        q.add(p);
        assert_eq!(q.take_due(1000)[0].tokens, None);
    }

    #[test]
    fn not_due_items_stay_and_due_ones_come_out() {
        let mut q = RetryQueue::new(60, 3);
        let mut later = pending(43200, 1000);
        later.id = "later".into();
        later.next_at = 2000;
        q.add(later);
        q.add(pending(43200, 1000));
        let due = q.take_due(1000);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, "a");
        assert_eq!(q.len(), 1);
    }
}
