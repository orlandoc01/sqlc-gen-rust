use std::collections::{BTreeMap, btree_map::Entry};

/// Inserts unless `key` is taken, returning the existing value so the caller can name it.
pub(crate) fn insert_unique<K: Ord, V: Clone>(
    map: &mut BTreeMap<K, V>,
    key: K,
    value: V,
) -> Result<(), V> {
    match map.entry(key) {
        Entry::Vacant(entry) => {
            entry.insert(value);
            Ok(())
        }
        Entry::Occupied(entry) => Err(entry.get().clone()),
    }
}
