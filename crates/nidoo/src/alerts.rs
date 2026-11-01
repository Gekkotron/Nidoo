//! The push alerts the hub knows: the last parsed history payload, kept in memory. The
//! broker's retained message is the store; this is only its decoded copy.

use std::hash::{DefaultHasher, Hash, Hasher};

use nidoo_api::{AlertView, parse_history};

use crate::media::proxied;

/// What applying a history payload did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Applied {
    /// The list changed; here it is, for the broadcast.
    Changed(Vec<AlertView>),
    /// The payload decoded to the list already held.
    Unchanged,
    /// The payload is not a JSON array; the list is kept. `first_time` is false when the same
    /// bad payload was seen before, so the caller logs it once.
    Invalid { first_time: bool },
}

#[derive(Debug, Default)]
pub struct AlertStore {
    list: Vec<AlertView>,
    last_bad_payload: Option<u64>,
    /// Media on this origin is rewritten to the hub's media route.
    media_origin: Option<String>,
}

impl AlertStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// A store that routes snapshot and clip URLs on `origin` through `/api/v1/media`.
    pub fn with_media_origin(origin: Option<String>) -> Self {
        Self {
            media_origin: origin,
            ..Self::default()
        }
    }

    /// Applies one history payload. An empty payload (the retained message was cleared)
    /// empties the list.
    pub fn apply(&mut self, payload: &[u8]) -> Applied {
        let next = if payload.is_empty() {
            Vec::new()
        } else {
            match parse_history(payload) {
                Some(mut list) => {
                    if let Some(origin) = &self.media_origin {
                        for alert in &mut list {
                            rewrite(&mut alert.image_url, origin);
                            rewrite(&mut alert.video_url, origin);
                        }
                    }
                    list
                }
                None => {
                    let mut hasher = DefaultHasher::new();
                    payload.hash(&mut hasher);
                    let hash = hasher.finish();
                    let first_time = self.last_bad_payload != Some(hash);
                    self.last_bad_payload = Some(hash);
                    return Applied::Invalid { first_time };
                }
            }
        };
        if next == self.list {
            return Applied::Unchanged;
        }
        self.list = next;
        Applied::Changed(self.list.clone())
    }

    pub fn list(&self) -> &[AlertView] {
        &self.list
    }

    pub fn find(&self, id: &str) -> Option<&AlertView> {
        self.list.iter().find(|a| a.id == id)
    }
}

fn rewrite(url: &mut Option<String>, origin: &str) {
    if let Some(proxied) = url.as_deref().and_then(|u| proxied(u, origin)) {
        *url = Some(proxied);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTRY: &[u8] = br#"[{"v":1,"id":"a","title":"A","sent_at":1}]"#;

    #[test]
    fn changed_then_unchanged() {
        let mut store = AlertStore::new();
        assert!(matches!(store.apply(ENTRY), Applied::Changed(list) if list.len() == 1));
        assert_eq!(store.apply(ENTRY), Applied::Unchanged);
        assert_eq!(store.find("a").map(|a| a.title.as_str()), Some("A"));
        assert!(store.find("b").is_none());
    }

    #[test]
    fn empty_payload_clears() {
        let mut store = AlertStore::new();
        store.apply(ENTRY);
        assert_eq!(store.apply(b""), Applied::Changed(vec![]));
        assert!(store.list().is_empty());
        assert_eq!(store.apply(b""), Applied::Unchanged);
    }

    #[test]
    fn apply_rewrites_media_on_the_frigate_origin() {
        let mut store = AlertStore::with_media_origin(Some("http://h:8971".into()));
        let payload = br#"[{"v":1,"id":"a","title":"A","sent_at":1,"image_url":"http://h:8971/api/events/1/snapshot.jpg","video_url":"http://h:8971/api/events/1/clip.mp4"},{"v":1,"id":"b","title":"B","sent_at":1,"image_url":"http://elsewhere/x.jpg"}]"#;
        store.apply(payload);
        let a = store.find("a").expect("a");
        assert_eq!(
            a.image_url.as_deref(),
            Some("/api/v1/media?url=http%3A%2F%2Fh%3A8971%2Fapi%2Fevents%2F1%2Fsnapshot.jpg")
        );
        assert_eq!(
            a.video_url.as_deref(),
            Some("/api/v1/media?url=http%3A%2F%2Fh%3A8971%2Fapi%2Fevents%2F1%2Fclip.mp4")
        );
        assert_eq!(
            store.find("b").and_then(|b| b.image_url.as_deref()),
            Some("http://elsewhere/x.jpg")
        );
    }

    #[test]
    fn invalid_payload_is_reported_once_per_payload() {
        let mut store = AlertStore::new();
        store.apply(ENTRY);
        assert_eq!(store.apply(b"{}"), Applied::Invalid { first_time: true });
        assert_eq!(store.apply(b"{}"), Applied::Invalid { first_time: false });
        assert_eq!(store.apply(b"nope"), Applied::Invalid { first_time: true });
        assert_eq!(store.list().len(), 1);
    }
}
