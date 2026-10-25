//! Spreading file work (reading headers, fingerprinting) over several threads.

use std::thread;

/// Like `items.iter().map(work).collect()`, but split over up to 8 threads.
/// The results come back in the same order as `items`. `work` must not panic.
pub fn map<T: Sync, R: Send>(items: &[T], work: impl Fn(&T) -> R + Sync) -> Vec<R> {
    if items.is_empty() {
        return Vec::new();
    }
    let threads = thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8);
    let chunk = items.len().div_ceil(threads);
    let work = &work;
    thread::scope(|scope| {
        let workers: Vec<_> = items
            .chunks(chunk)
            .map(|part| scope.spawn(move || part.iter().map(work).collect::<Vec<R>>()))
            .collect();
        workers.into_iter().flat_map(|worker| worker.join().expect("a worker thread panicked")).collect()
    })
}
