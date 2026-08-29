use super::probe::{self, Health};
use super::supervisor;
use super::tunnels::Forward;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

/// Identifies a rule by its place in the profile list.
pub type Slot = (usize, usize);

pub enum Job {
    Start(Slot, Forward),
    Stop(Slot, u32),
    /// Measure every rule that currently has a process behind it.
    Probe(Vec<(Slot, Forward)>),
}

pub enum Outcome {
    Started(Slot, Result<(), String>),
    Stopped(Slot, Result<(), String>),
    Probed(Vec<(Slot, Health)>),
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
                let outcome = match job {
                    Job::Start(slot, forward) => Outcome::Started(
                        slot,
                        supervisor::start(&forward).map_err(|e| e.to_string()),
                    ),
                    Job::Stop(slot, pid) => {
                        Outcome::Stopped(slot, supervisor::stop(pid).map_err(|e| e.to_string()))
                    }
                    Job::Probe(rules) => Outcome::Probed(
                        rules
                            .into_iter()
                            .map(|(slot, forward)| (slot, probe::check(&forward)))
                            .collect(),
                    ),
                };
                if outcome_tx.send(outcome).is_err() {
                    break;
                }
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
    use super::super::tunnels::Kind;
    use super::*;
    use std::time::{Duration, Instant};

    fn unreachable_forward() -> Forward {
        Forward {
            host: "kami".into(),
            kind: Kind::Local,
            // Port 0 is never a listener, so the probe resolves without waiting
            // on anything real.
            bind: "0".into(),
            target: "127.0.0.1:0".into(),
            note: String::new(),
        }
    }

    #[test]
    fn draining_an_idle_worker_returns_nothing_and_does_not_block() {
        let worker = Worker::spawn();
        let started = Instant::now();
        assert!(worker.drain().is_empty());
        assert!(started.elapsed() < Duration::from_millis(50));
    }

    #[test]
    fn a_probe_comes_back_on_the_channel() {
        let worker = Worker::spawn();
        worker.submit(Job::Probe(vec![((0, 0), unreachable_forward())]));

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(Outcome::Probed(results)) = worker.drain().into_iter().next() {
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].0, (0, 0));
                return;
            }
            assert!(Instant::now() < deadline, "probe never reported back");
            thread::sleep(Duration::from_millis(20));
        }
    }
}
