use std::hint::black_box;

use criterion::Criterion;
use criterion::criterion_group;
use criterion::criterion_main;
use tako_rs_core::queue::Queue;

fn dedup_lookup(c: &mut Criterion) {
  let runtime = tokio::runtime::Builder::new_current_thread()
    .build()
    .unwrap();
  let mut group = c.benchmark_group("queue_dedup");
  for size in [100, 10_000] {
    let queue = Queue::new();
    runtime.block_on(async {
      for id in 0..size {
        queue.push_dedup("job", &id, id.to_string()).await.unwrap();
      }
    });
    let key = (size - 1).to_string();
    group.bench_function(size.to_string(), |bencher| {
      bencher.iter(|| {
        runtime.block_on(async { black_box(queue.push_dedup("job", &42, &key).await.unwrap()) })
      });
    });
  }
  group.finish();
}

criterion_group!(benches, dedup_lookup);
criterion_main!(benches);
