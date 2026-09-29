//! The upstream URLs one playback session may fetch through the proxy.
//!
//! A manifest's segment, key and sub-playlist URIs are replaced by opaque ids
//! under the session's token (`/stream/{token}/r/{id}`), so no upstream URL —
//! an Xtream stream path carries the account's credentials — reaches the
//! player, and the proxy never fetches a URL that one of the session's own
//! manifests didn't name (no open relay).

use std::collections::{HashMap, VecDeque};
use std::hash::{BuildHasher, RandomState};

/// Enough for a multi-hour VOD playlist with short segments, variants and
/// keys; a live playlist re-uses ids, so it only grows as segments roll.
const MAX_RESOURCES: usize = 20_000;

pub(crate) struct ResourceTable {
    /// Per-session hash keys: ids are unguessable and differ per session.
    keys: RandomState,
    urls: HashMap<String, String>,
    /// Registration order, oldest first (evicted past [`MAX_RESOURCES`]).
    order: VecDeque<String>,
}

impl ResourceTable {
    pub(crate) fn new() -> Self {
        Self {
            keys: RandomState::new(),
            urls: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    /// The id for `url`, registering it if new. The same URL (a live
    /// manifest refetched) keeps its id.
    pub(crate) fn register(&mut self, url: &str) -> String {
        let id = format!("{:016x}", self.keys.hash_one(url));
        if !self.urls.contains_key(&id) {
            if self.order.len() >= MAX_RESOURCES {
                if let Some(oldest) = self.order.pop_front() {
                    self.urls.remove(&oldest);
                }
            }
            self.urls.insert(id.clone(), url.to_string());
            self.order.push_back(id.clone());
        }
        id
    }

    pub(crate) fn get(&self, id: &str) -> Option<&str> {
        self.urls.get(id).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_per_url_and_bounded() {
        let mut table = ResourceTable::new();
        let a = table.register("https://cdn/a.ts");
        assert_eq!(
            table.register("https://cdn/a.ts"),
            a,
            "a refetched manifest re-uses ids"
        );
        assert_ne!(table.register("https://cdn/b.ts"), a);
        assert_eq!(table.get(&a), Some("https://cdn/a.ts"));
        assert_eq!(table.get("0000000000000000"), None);
        assert!(!a.contains("cdn"), "ids never carry the URL");

        let other = ResourceTable::new();
        let _ = other; // another session hashes with other keys
        let mut big = ResourceTable::new();
        let first = big.register("u-0");
        for n in 1..=MAX_RESOURCES {
            big.register(&format!("u-{n}"));
        }
        assert_eq!(big.get(&first), None, "the oldest is evicted past the cap");
        assert_eq!(big.urls.len(), MAX_RESOURCES);
    }
}
