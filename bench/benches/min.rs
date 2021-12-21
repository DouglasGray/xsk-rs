use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::cmp;

fn min_cmp(fst: usize, snd: usize) -> usize {
    cmp::min(fst, snd)
}

fn min_usize(fst: usize, snd: usize) -> usize {
    if fst < snd {
        fst
    } else {
        snd
    }
}

fn bench_min(c: &mut Criterion) {
    let mut group = c.benchmark_group("min");

    for vals in [(1, 0), (0, 1), (1, 1)] {
        let p = format!("({}, {})", vals.0, vals.1);

        group.bench_with_input(BenchmarkId::new("cmp", p.clone()), &vals, |b, vals| {
            b.iter(|| min_cmp(vals.0, vals.1));
        });

        group.bench_with_input(BenchmarkId::new("if_else", p), &vals, |b, vals| {
            b.iter(|| min_usize(vals.0, vals.1));
        });
    }

    group.finish();
}

criterion_group!(benches, bench_min);
criterion_main!(benches);
