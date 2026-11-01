//! The latest picture of each camera or image entity, kept in memory.

use std::collections::HashMap;
use std::sync::Mutex;

use bytes::Bytes;

/// The most a picture may weigh, received or fetched.
pub const MAX_PICTURE_BYTES: usize = 5 * 1024 * 1024;

/// What the hub knows about an entity's picture.
#[derive(Clone, Debug)]
pub enum Picture {
    Bytes { content_type: String, body: Bytes },
    Url(String),
}

#[derive(Debug, Default)]
pub struct Pictures {
    inner: Mutex<HashMap<String, Picture>>,
}

impl Pictures {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&self, id: &str, picture: Picture) {
        self.lock().insert(id.to_owned(), picture);
    }

    pub fn get(&self, id: &str) -> Option<Picture> {
        self.lock().get(id).cloned()
    }

    pub fn clear(&self, id: &str) {
        self.lock().remove(id);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Picture>> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
