use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

use super::page::Node;
use crate::domain::PageId;
use crate::{Error, PageCacheStats, Result};

struct CacheEntry {
    node: Arc<Node>,
    last_access: AtomicU64,
}

const MAX_CACHE_SHARDS: usize = 64;
const LOCAL_CACHE_SLOTS: usize = 64;
const LOCAL_CACHE_WAYS: usize = 8;
const LOCAL_CACHE_SETS: usize = LOCAL_CACHE_SLOTS / LOCAL_CACHE_WAYS;
const ACCESS_SAMPLE_INTERVAL: u8 = 64;
static NEXT_METRIC_SLOT: AtomicUsize = AtomicUsize::new(0);
static NEXT_CACHE_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static METRIC_SLOT: usize = NEXT_METRIC_SLOT.fetch_add(1, Ordering::Relaxed)
        % MAX_CACHE_SHARDS;
    static ACCESS_SAMPLE: Cell<u8> = const { Cell::new(0) };
    static LOCAL_CACHE: RefCell<LocalCache> = RefCell::new(LocalCache::new());
}

struct LocalEntry {
    cache_id: u64,
    page_id: PageId,
    generation: u64,
    node: Arc<Node>,
}

struct LocalCache {
    entries: [Option<LocalEntry>; LOCAL_CACHE_SLOTS],
    next_victim: [usize; LOCAL_CACHE_SETS],
}

impl LocalCache {
    fn new() -> Self {
        Self {
            entries: std::array::from_fn(|_| None),
            next_victim: [0; LOCAL_CACHE_SETS],
        }
    }
}

struct CacheShard {
    capacity: usize,
    clock: AtomicU64,
    generation: AtomicU64,
    pages: RwLock<BTreeMap<PageId, CacheEntry>>,
}

#[repr(align(64))]
struct Counter(AtomicU64);

pub(crate) struct PageCache {
    id: u64,
    capacity: usize,
    hits: [Counter; MAX_CACHE_SHARDS],
    misses: [Counter; MAX_CACHE_SHARDS],
    shards: Vec<CacheShard>,
}

impl PageCache {
    pub(crate) fn new(capacity: usize) -> Self {
        let shard_count = if capacity < MAX_CACHE_SHARDS {
            1
        } else {
            (capacity / MAX_CACHE_SHARDS).clamp(1, MAX_CACHE_SHARDS)
        };
        let base_capacity = capacity / shard_count;
        let remainder = capacity % shard_count;
        Self {
            id: NEXT_CACHE_ID.fetch_add(1, Ordering::Relaxed),
            capacity,
            hits: std::array::from_fn(|_| Counter(AtomicU64::new(0))),
            misses: std::array::from_fn(|_| Counter(AtomicU64::new(0))),
            shards: (0..shard_count)
                .map(|index| CacheShard {
                    capacity: base_capacity + usize::from(index < remainder),
                    clock: AtomicU64::new(0),
                    generation: AtomicU64::new(0),
                    pages: RwLock::new(BTreeMap::new()),
                })
                .collect(),
        }
    }

    pub(crate) fn get(&self, page_id: PageId) -> Result<Option<Arc<Node>>> {
        let shard = self.shard(page_id);
        let generation = shard.generation.load(Ordering::Acquire);
        if let Some(node) = local_get(self.id, page_id, generation) {
            increment_counter(&self.hits);
            return Ok(Some(node));
        }
        let pages = shard
            .pages
            .read()
            .map_err(|_| Error::LockPoisoned("page cache"))?;
        let Some(entry) = pages.get(&page_id) else {
            increment_counter(&self.misses);
            return Ok(None);
        };
        if self.shards.len() == 1 || sample_access() {
            entry.last_access.store(
                shard.clock.fetch_add(1, Ordering::Relaxed),
                Ordering::Relaxed,
            );
        }
        increment_counter(&self.hits);
        let node = Arc::clone(&entry.node);
        drop(pages);
        local_insert(self.id, page_id, generation, Arc::clone(&node));
        Ok(Some(node))
    }

    pub(crate) fn inspect<T>(
        &self,
        page_id: PageId,
        load: impl FnOnce() -> Result<Arc<Node>>,
        inspect: impl FnOnce(&Node) -> Result<T>,
    ) -> Result<T> {
        let shard = self.shard(page_id);
        let generation = shard.generation.load(Ordering::Acquire);
        let inspect = match local_inspect(self.id, page_id, generation, inspect) {
            Ok(result) => {
                increment_counter(&self.hits);
                return result;
            }
            Err(inspect) => inspect,
        };
        let pages = shard
            .pages
            .read()
            .map_err(|_| Error::LockPoisoned("page cache"))?;
        if let Some(entry) = pages.get(&page_id) {
            if self.shards.len() == 1 || sample_access() {
                entry.last_access.store(
                    shard.clock.fetch_add(1, Ordering::Relaxed),
                    Ordering::Relaxed,
                );
            }
            increment_counter(&self.hits);
            let result = inspect(&entry.node);
            let node = Arc::clone(&entry.node);
            drop(pages);
            local_insert(self.id, page_id, generation, node);
            return result;
        }
        increment_counter(&self.misses);
        drop(pages);
        let node = load()?;
        let result = inspect(&node);
        self.insert(page_id, node)?;
        result
    }

    pub(crate) fn insert(&self, page_id: PageId, node: Arc<Node>) -> Result<()> {
        if self.capacity == 0 {
            return Ok(());
        }
        let shard = self.shard(page_id);
        let mut pages = shard
            .pages
            .write()
            .map_err(|_| Error::LockPoisoned("page cache"))?;
        pages.insert(
            page_id,
            CacheEntry {
                node,
                last_access: AtomicU64::new(shard.clock.fetch_add(1, Ordering::Relaxed)),
            },
        );
        while pages.len() > shard.capacity {
            let evicted = pages
                .iter()
                .min_by_key(|(_, entry)| entry.last_access.load(Ordering::Relaxed))
                .map(|(page_id, _)| *page_id)
                .ok_or(Error::InternalInvariant("page cache eviction is empty"))?;
            pages.remove(&evicted);
        }
        shard.generation.fetch_add(1, Ordering::Release);
        Ok(())
    }

    pub(crate) fn stats(&self) -> Result<PageCacheStats> {
        let mut resident_pages = 0_usize;
        for shard in &self.shards {
            resident_pages = resident_pages
                .checked_add(
                    shard
                        .pages
                        .read()
                        .map_err(|_| Error::LockPoisoned("page cache"))?
                        .len(),
                )
                .ok_or(Error::InternalInvariant("page cache residency overflow"))?;
        }
        Ok(PageCacheStats {
            capacity: self.capacity,
            resident_pages,
            hits: sum_counters(&self.hits),
            misses: sum_counters(&self.misses),
        })
    }

    fn shard(&self, page_id: PageId) -> &CacheShard {
        let shard_count = u64::try_from(self.shards.len()).unwrap_or(1);
        let index = usize::try_from(page_id.value() % shard_count).unwrap_or(0);
        &self.shards[index]
    }
}

fn increment_counter(counters: &[Counter; MAX_CACHE_SHARDS]) {
    METRIC_SLOT.with(|slot| {
        counters[*slot].0.fetch_add(1, Ordering::Relaxed);
    });
}

fn sum_counters(counters: &[Counter; MAX_CACHE_SHARDS]) -> u64 {
    counters.iter().fold(0_u64, |total, counter| {
        total.saturating_add(counter.0.load(Ordering::Relaxed))
    })
}

fn sample_access() -> bool {
    ACCESS_SAMPLE.with(|sample| {
        let current = sample.get();
        sample.set(current.wrapping_add(1));
        current % ACCESS_SAMPLE_INTERVAL == 0
    })
}

fn local_get(cache_id: u64, page_id: PageId, generation: u64) -> Option<Arc<Node>> {
    LOCAL_CACHE.with(|cache| {
        let cache = cache.borrow();
        let first = local_set(page_id) * LOCAL_CACHE_WAYS;
        cache.entries[first..first + LOCAL_CACHE_WAYS]
            .iter()
            .flatten()
            .find(|entry| {
                entry.cache_id == cache_id
                    && entry.page_id == page_id
                    && entry.generation == generation
            })
            .map(|entry| Arc::clone(&entry.node))
    })
}

fn local_inspect<T, F>(
    cache_id: u64,
    page_id: PageId,
    generation: u64,
    inspect: F,
) -> std::result::Result<Result<T>, F>
where
    F: FnOnce(&Node) -> Result<T>,
{
    LOCAL_CACHE.with(|cache| {
        let cache = cache.borrow();
        let first = local_set(page_id) * LOCAL_CACHE_WAYS;
        let entry = cache.entries[first..first + LOCAL_CACHE_WAYS]
            .iter()
            .flatten()
            .find(|entry| {
                entry.cache_id == cache_id
                    && entry.page_id == page_id
                    && entry.generation == generation
            });
        match entry {
            Some(entry) => Ok(inspect(&entry.node)),
            None => Err(inspect),
        }
    })
}

fn local_insert(cache_id: u64, page_id: PageId, generation: u64, node: Arc<Node>) {
    LOCAL_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let set = local_set(page_id);
        let first = set * LOCAL_CACHE_WAYS;
        let matching = cache.entries[first..first + LOCAL_CACHE_WAYS]
            .iter()
            .position(|entry| {
                entry
                    .as_ref()
                    .is_some_and(|entry| entry.cache_id == cache_id && entry.page_id == page_id)
            });
        let way = matching.unwrap_or_else(|| {
            let way = cache.next_victim[set];
            cache.next_victim[set] = (way + 1) % LOCAL_CACHE_WAYS;
            way
        });
        cache.entries[first + way] = Some(LocalEntry {
            cache_id,
            page_id,
            generation,
            node,
        });
    });
}

fn local_set(page_id: PageId) -> usize {
    let sets = u64::try_from(LOCAL_CACHE_SETS).unwrap_or(1);
    usize::try_from(page_id.value() % sets).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::Arc;

    use super::{LOCAL_CACHE_SETS, PageCache, local_get, local_insert};
    use crate::domain::PageId;
    use crate::infrastructure::page::{LeafEntry, LeafValue, Node};

    #[test]
    fn least_recently_used_page_is_evicted() {
        let cache = PageCache::new(2);
        let first = PageId::from_raw(2).unwrap();
        let second = PageId::from_raw(3).unwrap();
        let third = PageId::from_raw(4).unwrap();
        cache.insert(first, Arc::new(node(b"first"))).unwrap();
        cache.insert(second, Arc::new(node(b"second"))).unwrap();
        assert!(cache.get(first).unwrap().is_some());
        cache.insert(third, Arc::new(node(b"third"))).unwrap();

        assert!(cache.get(second).unwrap().is_none());
        assert!(cache.get(first).unwrap().is_some());
        assert!(cache.get(third).unwrap().is_some());
        assert_eq!(cache.stats().unwrap().resident_pages, 2);
    }

    #[test]
    fn sharded_hits_remain_exact_under_concurrent_reads() {
        let cache = Arc::new(PageCache::new(4096));
        for raw in 2..258_u64 {
            cache
                .insert(PageId::from_raw(raw).unwrap(), Arc::new(node(b"entry")))
                .unwrap();
        }

        std::thread::scope(|scope| {
            for worker in 0..16_u64 {
                let cache = Arc::clone(&cache);
                scope.spawn(move || {
                    for operation in 0..1_000_u64 {
                        let raw = 2 + (operation + worker) % 256;
                        assert!(cache.get(PageId::from_raw(raw).unwrap()).unwrap().is_some());
                    }
                });
            }
        });

        let stats = cache.stats().unwrap();
        assert_eq!(stats.hits, 16_000);
        assert_eq!(stats.misses, 0);
        assert_eq!(stats.resident_pages, 256);
    }

    #[test]
    fn local_cache_retains_eight_colliding_pages() {
        let cache_id = u64::MAX;
        let generation = 17;
        let pages = (0..8_u64)
            .map(|index| {
                PageId::from_raw(2 + index * u64::try_from(LOCAL_CACHE_SETS).unwrap()).unwrap()
            })
            .collect::<Vec<_>>();

        for page_id in &pages {
            local_insert(cache_id, *page_id, generation, Arc::new(node(b"entry")));
        }
        for page_id in pages {
            assert!(local_get(cache_id, page_id, generation).is_some());
        }
    }

    #[test]
    fn shared_insert_invalidates_an_older_local_entry() {
        let cache = PageCache::new(2);
        let page_id = PageId::from_raw(2).unwrap();
        let old = node(b"old");
        let new = node(b"new");
        cache.insert(page_id, Arc::new(old.clone())).unwrap();
        assert_eq!(cache.get(page_id).unwrap().unwrap().as_ref(), &old);

        cache.insert(page_id, Arc::new(new.clone())).unwrap();

        assert_eq!(cache.get(page_id).unwrap().unwrap().as_ref(), &new);
    }

    #[test]
    fn inspect_loads_a_missing_page_once() {
        let cache = PageCache::new(2);
        let page_id = PageId::from_raw(2).unwrap();
        let loads = Cell::new(0_u32);
        let inspect = |node: &Node| match node {
            Node::Leaf(entries) => Ok(entries[0].key.to_vec()),
            _ => panic!("expected a leaf"),
        };

        let first = cache
            .inspect(
                page_id,
                || {
                    loads.set(loads.get() + 1);
                    Ok(Arc::new(node(b"first")))
                },
                inspect,
            )
            .unwrap();
        let second = cache
            .inspect(
                page_id,
                || {
                    loads.set(loads.get() + 1);
                    Ok(Arc::new(node(b"wrong")))
                },
                inspect,
            )
            .unwrap();

        assert_eq!(first, b"first");
        assert_eq!(second, b"first");
        assert_eq!(loads.get(), 1);
    }

    fn node(key: &[u8]) -> Node {
        Node::Leaf(vec![LeafEntry {
            key: key.into(),
            value: LeafValue::Inline(Box::default()),
        }])
    }
}
