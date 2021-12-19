use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use rand::Rng;
use std::cmp;

fn min_cmp(maybe_smaller: usize, maybe_larger: usize) -> usize {
    cmp::min(maybe_smaller, maybe_larger)
}

fn min_usize(maybe_smaller: usize, maybe_larger: usize) -> usize {
    if maybe_smaller < maybe_larger {
        maybe_smaller
    } else {
        maybe_larger
    }
}

fn bench_min(c: &mut Criterion) {
    let num = 1_000_000;
    let mut rng = rand::thread_rng();

    // There are a lot of calls to cmp::min where it's likely one of
    // them is larger than the other, for example a cursor position
    // and the length of the underlying buffer. Want to check how
    // much difference a manual impl where the first branch is
    // usually hit would make.
    let smaller: Vec<_> = (0..num).into_iter().map(|_| rng.gen::<usize>()).collect();
    let larger: Vec<_> = smaller.iter().map(|i| i + rng.gen::<usize>()).collect();

    let pairs: Vec<(usize, usize)> = smaller.into_iter().zip(larger.into_iter()).collect();

    let mut group = c.benchmark_group("min calcs");

    group.bench_with_input(BenchmarkId::new("min_cmp", num), &pairs, |b, pairs| {
        b.iter(|| {
            let mut x = 0;
            for (fst, snd) in pairs {
                x = min_cmp(*fst, *snd);
            }
            criterion::black_box(x);
        });
    });

    group.bench_with_input(BenchmarkId::new("min_usize", num), &pairs, |b, pairs| {
        b.iter(|| {
            let mut x = 0;
            for (fst, snd) in pairs {
                x = min_usize(*fst, *snd);
            }
            criterion::black_box(x);
        });
    });

    group.finish();
}

criterion_group!(benches, bench_min);
criterion_main!(benches);
