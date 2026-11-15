//! The texture set behind [`super::Artwork`]: least recently used first out,
//! held to a count and a GPU byte budget.

use std::collections::HashMap;

use egui::TextureHandle;

use super::TEXTURE_BUDGET;

/// Key-ordered texture set: the least-recently-used entry is freed first.
pub(super) struct Lru {
    cap: usize,
    budget: usize,
    bytes: usize,
    clock: u64,
    entries: HashMap<String, Entry>,
}

struct Entry {
    handle: TextureHandle,
    used: u64,
    bytes: usize,
}

/// The GPU bytes one RGBA texture takes with its mip chain (~4/3 of the base).
pub(super) fn texture_bytes(handle: &TextureHandle) -> usize {
    let [w, h] = handle.size();
    w * h * 4 * 4 / 3
}

impl Lru {
    pub(super) fn new(cap: usize) -> Self {
        Self::with_budget(cap, TEXTURE_BUDGET)
    }

    pub(super) fn with_budget(cap: usize, budget: usize) -> Self {
        Self {
            cap: cap.max(1),
            budget,
            bytes: 0,
            clock: 0,
            entries: HashMap::new(),
        }
    }

    pub(super) fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    /// Free every texture: dropping a [`TextureHandle`] is what releases it.
    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    /// Marks `key` most-recently-used and returns its texture.
    pub(super) fn get(&mut self, key: &str) -> Option<&TextureHandle> {
        let used = self.tick();
        self.entries.get_mut(key).map(|entry| {
            entry.used = used;
            &entry.handle
        })
    }

    /// Inserts `handle`, freeing the least-recently-used textures until both
    /// the count cap and the byte budget hold; returns how many were freed.
    pub(super) fn insert(&mut self, key: String, handle: TextureHandle) -> usize {
        let used = self.tick();
        let bytes = texture_bytes(&handle);
        if let Some(old) = self.entries.remove(&key) {
            self.bytes -= old.bytes;
        }
        let mut freed = 0;
        while !self.entries.is_empty()
            && (self.entries.len() >= self.cap || self.bytes + bytes > self.budget)
        {
            let Some(victim) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(entry) = self.entries.remove(&victim) {
                self.bytes -= entry.bytes;
                freed += 1;
            }
        }
        self.bytes += bytes;
        self.entries.insert(
            key,
            Entry {
                handle,
                used,
                bytes,
            },
        );
        freed
    }

    /// How many textures are held.
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    /// The GPU bytes the held textures take.
    #[cfg(test)]
    pub(super) fn bytes(&self) -> usize {
        self.bytes
    }

    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }
}
