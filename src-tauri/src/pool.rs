//! A tiny fixed-size worker pool that runs the *newest* job first.
//!
//! When you scroll quickly through a big folder, the grid asks for many
//! thumbnails that scroll off-screen again before they are ready. Running the
//! most recent request first means the thumbnails you are looking at right now
//! get made before the ones you already scrolled past.

use std::sync::{Arc, Condvar, Mutex};
use std::thread;

type Job = Box<dyn FnOnce() + Send + 'static>;

#[derive(Default)]
struct Queue {
    jobs: Mutex<Vec<Job>>,
    ready: Condvar,
}

pub struct WorkerPool {
    queue: Arc<Queue>,
}

impl WorkerPool {
    pub fn new(threads: usize) -> Self {
        let queue = Arc::new(Queue::default());
        for i in 0..threads {
            let queue = queue.clone();
            thread::Builder::new()
                .name(format!("thumb-worker-{i}"))
                .spawn(move || loop {
                    let job = {
                        let mut jobs = queue.jobs.lock().unwrap();
                        loop {
                            // `pop` takes from the end of the list: newest first.
                            if let Some(job) = jobs.pop() {
                                break job;
                            }
                            jobs = queue.ready.wait(jobs).unwrap();
                        }
                    };
                    job();
                })
                .expect("failed to start thumbnail worker thread");
        }
        Self { queue }
    }

    pub fn submit(&self, job: impl FnOnce() + Send + 'static) {
        self.queue.jobs.lock().unwrap().push(Box::new(job));
        self.queue.ready.notify_one();
    }
}
