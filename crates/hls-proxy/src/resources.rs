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
    /// id → (url, last use).
    urls: HashMap<String, (String, u64)>,
    /// Uses, oldest first; an entry whose stamp is older than its URL's
    /// last use is stale and skipped.
    uses: VecDeque<(String, u64)>,
    clock: u64,
}

impl ResourceTable {
    pub(crate) fn new() -> Self {
        Self {
            keys: RandomState::new(),
            urls: HashMap::new(),
            uses: VecDeque::new(),
            clock: 0,
        }
    }

    /// The id for `url`, registering it if new. The same URL (a live
    /// manifest refetched) keeps its id and counts as used.
    pub(crate) fn register(&mut self, url: &str) -> String {
        let id = format!("{:016x}", self.keys.hash_one(url));
        if self.urls.contains_key(&id) {
            self.touch(&id);
            return id;
        }
        while self.urls.len() >= MAX_RESOURCES {
            self.evict_least_recent();
        }
        self.clock += 1;
        self.urls.insert(id.clone(), (url.to_string(), self.clock));
        self.uses.push_back((id.clone(), self.clock));
        id
    }

    /// The URL behind `id` (a use: the least recently used go first, so a
    /// live channel's variant playlists and key are never evicted).
    pub(crate) fn get(&mut self, id: &str) -> Option<String> {
        let url = self.urls.get(id)?.0.clone();
        self.touch(id);
        Some(url)
    }

    fn touch(&mut self, id: &str) {
        self.clock += 1;
        if let Some(entry) = self.urls.get_mut(id) {
            entry.1 = self.clock;
            self.uses.push_back((id.to_string(), self.clock));
        }
        // Stale stamps pile up with every use: rebuild once they dominate.
        if self.uses.len() > 4 * MAX_RESOURCES.max(self.urls.len()) {
            let mut live: Vec<(String, u64)> = self
                .urls
                .iter()
                .map(|(id, (_, stamp))| (id.clone(), *stamp))
                .collect();
            live.sort_by_key(|(_, stamp)| *stamp);
            self.uses = live.into();
        }
    }

    fn evict_least_recent(&mut self) {
        while let Some((id, stamp)) = self.uses.pop_front() {
            if self.urls.get(&id).is_some_and(|(_, last)| *last == stamp) {
                self.urls.remove(&id);
                return;
            }
        }
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
        assert_eq!(table.get(&a).as_deref(), Some("https://cdn/a.ts"));
        assert_eq!(table.get("0000000000000000"), None);
        assert!(!a.contains("cdn"), "ids never carry the URL");
    }

    #[test]
    fn what_a_live_stream_keeps_using_is_never_evicted() {
        let mut table = ResourceTable::new();
        // Registered first: the variant playlist and the key.
        let variant = table.register("https://cdn/variant.m3u8");
        let key = table.register("https://cdn/key.bin");
        let first_segment = table.register("u-0");
        // Hours of rolling segments, the playlist refetched and the key
        // re-registered along the way.
        for n in 1..=3 * MAX_RESOURCES {
            table.register(&format!("u-{n}"));
            if n % 3 == 0 {
                assert!(table.get(&variant).is_some(), "variant evicted at {n}");
                table.register("https://cdn/key.bin");
            }
        }
        assert_eq!(table.get(&key).as_deref(), Some("https://cdn/key.bin"));
        assert_eq!(table.get(&first_segment), None, "old segments go");
        assert_eq!(table.urls.len(), MAX_RESOURCES);
        assert!(table.uses.len() <= 4 * MAX_RESOURCES + 1);
    }
}
