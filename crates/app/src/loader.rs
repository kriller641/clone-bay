//! Reads settings files on background threads so the window never freezes.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;

use clone_bay_core::inspect::{inspect_file, FileSummary, InspectError};

/// More threads than this don't help: the work is dominated by disk reads.
const MAX_WORKERS: usize = 4;

/// The outcome of reading one file.
pub struct Inspected {
    /// Which batch this belongs to; results from an older batch are stale.
    pub batch: u64,
    /// The position the file was given with when the batch was started.
    pub index: usize,
    pub result: Result<FileSummary, InspectError>,
}

pub struct Loader {
    /// The batch currently wanted. Workers stop when it moves on.
    current: Arc<AtomicU64>,
    sender: Sender<Inspected>,
    receiver: Receiver<Inspected>,
}

impl Loader {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            current: Arc::new(AtomicU64::new(0)),
            sender,
            receiver,
        }
    }

    /// Starts reading `files`, each given with its position in the list,
    /// abandoning any batch still in progress. `wake` is called after each
    /// file so the window can redraw.
    pub fn start(
        &self,
        files: Vec<(usize, PathBuf)>,
        wake: impl Fn() + Send + Clone + 'static,
    ) -> u64 {
        let batch = self.current.fetch_add(1, Ordering::SeqCst) + 1;
        let workers = thread::available_parallelism()
            .map_or(1, usize::from)
            .min(MAX_WORKERS)
            .min(files.len());
        let files = Arc::new(files);
        let next = Arc::new(AtomicUsize::new(0));
        for _ in 0..workers {
            let (files, next) = (Arc::clone(&files), Arc::clone(&next));
            let (current, sender, wake) =
                (Arc::clone(&self.current), self.sender.clone(), wake.clone());
            thread::spawn(move || {
                while current.load(Ordering::SeqCst) == batch {
                    let Some((index, path)) = files.get(next.fetch_add(1, Ordering::SeqCst)) else {
                        break;
                    };
                    let (index, result) = (*index, inspect_file(path));
                    let sent = sender.send(Inspected {
                        batch,
                        index,
                        result,
                    });
                    if sent.is_err() {
                        // The window has gone; nobody wants the rest.
                        break;
                    }
                    wake();
                }
            });
        }
        batch
    }

    /// Results that have arrived since the last call, for `batch` only.
    pub fn drain(&self, batch: u64) -> Vec<Inspected> {
        self.receiver
            .try_iter()
            .filter(|inspected| inspected.batch == batch)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn wait_for(loader: &Loader, batch: u64, count: usize) -> Vec<Inspected> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut got = Vec::new();
        while got.len() < count {
            assert!(Instant::now() < deadline, "timed out waiting for results");
            got.extend(loader.drain(batch));
            thread::sleep(Duration::from_millis(5));
        }
        got
    }

    #[test]
    fn reports_every_file_once_with_its_index() {
        let loader = Loader::new();
        let files: Vec<(usize, PathBuf)> = (0..9)
            .map(|i| (i, PathBuf::from(format!("missing_{i}.dat"))))
            .collect();
        let batch = loader.start(files, || {});
        let mut indices: Vec<usize> = wait_for(&loader, batch, 9)
            .into_iter()
            .map(|inspected| {
                assert!(matches!(inspected.result, Err(InspectError::Read(_))));
                inspected.index
            })
            .collect();
        indices.sort_unstable();
        assert_eq!(indices, (0..9).collect::<Vec<_>>());
        // Nothing more arrives once every file is reported.
        thread::sleep(Duration::from_millis(50));
        assert!(loader.drain(batch).is_empty());
    }

    #[test]
    fn results_from_an_abandoned_batch_are_dropped() {
        let loader = Loader::new();
        let old = loader.start(vec![(0, PathBuf::from("old.dat"))], || {});
        let new = loader.start(vec![(0, PathBuf::from("new.dat"))], || {});
        assert_ne!(old, new);
        let got = wait_for(&loader, new, 1);
        assert_eq!(got.len(), 1);
        assert!(got.iter().all(|inspected| inspected.batch == new));
    }

    #[test]
    fn results_carry_the_position_each_file_was_given_with() {
        let loader = Loader::new();
        let files = vec![(7, PathBuf::from("a.dat")), (3, PathBuf::from("b.dat"))];
        let batch = loader.start(files, || {});
        let mut indices: Vec<usize> = wait_for(&loader, batch, 2)
            .into_iter()
            .map(|inspected| inspected.index)
            .collect();
        indices.sort_unstable();
        assert_eq!(indices, [3, 7]);
    }

    #[test]
    fn an_empty_batch_starts_no_work() {
        let loader = Loader::new();
        let batch = loader.start(Vec::new(), || {});
        thread::sleep(Duration::from_millis(20));
        assert!(loader.drain(batch).is_empty());
    }

    #[test]
    fn wakes_the_window_after_each_file() {
        let loader = Loader::new();
        let woken = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&woken);
        let files = vec![(0, PathBuf::from("a.dat")), (1, PathBuf::from("b.dat"))];
        let batch = loader.start(files, move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        wait_for(&loader, batch, 2);
        let deadline = Instant::now() + Duration::from_secs(30);
        while woken.load(Ordering::SeqCst) < 2 {
            assert!(Instant::now() < deadline, "timed out waiting for wake-ups");
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(woken.load(Ordering::SeqCst), 2);
    }
}
