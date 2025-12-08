//! Benchmarks for indexing performance.

use criterion::{criterion_group, criterion_main, Criterion};

fn indexing_benchmark(_c: &mut Criterion) {
    // TODO: Implement benchmarks
}

criterion_group!(benches, indexing_benchmark);
criterion_main!(benches);
