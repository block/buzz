//! Retained additional prepared allocation capacity; excludes allocator metadata.
use buzz_core::prepared_filter::{prepare_filters, FilterInput};
use relay_filter_baseline::fanout;
use std::mem::size_of;
fn capacity(filters: &Vec<FilterInput>) -> (usize, usize) {
    fn add<T>(v: &Vec<T>, bytes: &mut usize, allocs: &mut usize) {
        *bytes += v.capacity() * size_of::<T>();
        *allocs += usize::from(v.capacity() != 0);
    }
    let (mut bytes, mut allocs) = (0, 0);
    add(filters, &mut bytes, &mut allocs);
    for f in filters {
        if let Some(ids) = &f.ids {
            add(ids, &mut bytes, &mut allocs);
        }
        if let Some(authors) = &f.authors {
            add(authors, &mut bytes, &mut allocs);
        }
        if let Some(kinds) = &f.kinds {
            add(kinds, &mut bytes, &mut allocs);
        }
        add(&f.tags, &mut bytes, &mut allocs);
        for t in &f.tags {
            add(&t.key, &mut bytes, &mut allocs);
            add(&t.values, &mut bytes, &mut allocs);
            for value in &t.values {
                add(value, &mut bytes, &mut allocs);
            }
        }
    }
    (bytes + size_of::<Vec<FilterInput>>(), allocs)
}
fn main() {
    let rows:Vec<_>=fanout::cases().iter().filter(|c|c.count==1 && c.case.name.contains("_first_late"))
        .map(|c|{let prepared=prepare_filters(&c.case.filters);let(bytes,allocs)=capacity(&prepared);
            serde_json::json!({"case":c.case.name,"additional_retained_bytes":bytes,"additional_allocations":allocs,
            "1000_subscriptions_bytes":bytes*1000})}).collect();
    println!("{}",serde_json::to_string_pretty(&serde_json::json!({"method":"exact Vec capacity times element size plus the new entry Vec header; excludes allocator metadata/bin rounding and event projection transient allocations","rows":rows})).unwrap());
}
