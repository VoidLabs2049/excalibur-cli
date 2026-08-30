use super::discover::{self, Listener};
use super::supervisor;
use super::tunnels::Forward;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

/// Identifies a rule by its place in the profile list.
pub type Slot = (usize, usize);

pub enum Job {
    Start(Slot, Forward),
    Stop(Slot, u32),
    /// Stop a process no rule claims, which therefore has no slot to name it by.
    StopOrphan(u32),
    /// Ask a host what it is listening on. A real network round trip, so it
    /// belongs here more than anything else in this enum.
    Discover(String),
}

pub enum Outcome {
    Started(Slot, Result<(), String>),
    Stopped(Slot, Result<(), String>),
    StoppedOrphan(u32, Result<(), String>),
    Discovered(String, Result<Vec<Listener>, String>),
}

/// Runs the blocking work off the render thread.
///
/// Starting a tunnel waits on ssh and the end-to-end probe waits on a TCP
/// round trip; either would freeze the UI for seconds if run inline, and a
/// frozen TUI is indistinguishable from a hung one.
#[derive(Debug)]
pub struct Worker {
    jobs: Sender<Job>,
    outcomes: Receiver<Outcome>,
}

impl Worker {
    pub fn spawn() -> Self {
        let (jobs, job_rx) = mpsc::channel::<Job>();
        let (outcome_tx, outcomes) = mpsc::channel::<Outcome>();

        thread::spawn(move || {
            // Ends when the owning state drops its sender, so re-entering the
            // module does not pile up threads.
            while let Ok(job) = job_rx.recv() {
                let tx = outcome_tx.clone();
                thread::spawn(move || {
                    let outcome = match job {
                        Job::Start(slot, forward) => Outcome::Started(
                            slot,
                            supervisor::start(&forward).map_err(|e| e.to_string()),
                        ),
                        Job::Stop(slot, pid) => {
                            Outcome::Stopped(slot, supervisor::stop(pid).map_err(|e| e.to_string()))
                        }
                        Job::StopOrphan(pid) => Outcome::StoppedOrphan(
                            pid,
                            supervisor::stop(pid).map_err(|e| e.to_string()),
                        ),
                        Job::Discover(host) => {
                            let found = discover::listeners(&host).map_err(|e| e.to_string());
                            Outcome::Discovered(host, found)
                        }
                    };
                    let _ = tx.send(outcome);
                });
            }
        });

        Worker { jobs, outcomes }
    }

    pub fn submit(&self, job: Job) {
        let _ = self.jobs.send(job);
    }

    /// Everything finished since the last call. Never blocks.
    pub fn drain(&self) -> Vec<Outcome> {
        let mut done = Vec::new();
        loop {
            match self.outcomes.try_recv() {
                Ok(outcome) => done.push(outcome),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return done,
            }
        }
    }
}

impl Default for Worker {
    fn default() -> Self {
        Worker::spawn()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn draining_an_idle_worker_returns_nothing_and_does_not_block() {
        let worker = Worker::spawn();
        let started = Instant::now();
        assert!(worker.drain().is_empty());
        assert!(started.elapsed() < Duration::from_millis(50));
    }
}
