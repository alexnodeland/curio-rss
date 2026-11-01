//! The lane queue a sweep hands its jobs out from — the feeds of a sweep
//! and the pages of its full-text hydration queue alike.

use std::collections::{HashMap, VecDeque};

/// One job handed out by a [`WorkQueue`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Work<T> {
    pub(super) job: T,
    /// Its host lane in the queue.
    pub(super) lane: usize,
}

/// A work list handed out fairly across hosts — a sweep's feeds, the
/// full-text hydration queue's pages. Each politeness-paced host gets a
/// lane with at most one job in flight; the jobs of unpaced hosts (and of
/// unknown URLs) share one lane without that cap. The lanes take turns,
/// each keeping its jobs in the order they were queued.
#[derive(Debug)]
pub(super) struct WorkQueue<T> {
    lanes: Vec<Lane<T>>,
    by_host: HashMap<Option<String>, usize>,
    /// The lane the next [`WorkQueue::take`] looks at first.
    turn: usize,
}

impl<T> Default for WorkQueue<T> {
    fn default() -> Self {
        Self {
            lanes: Vec::new(),
            by_host: HashMap::new(),
            turn: 0,
        }
    }
}

#[derive(Debug)]
struct Lane<T> {
    pending: VecDeque<Work<T>>,
    /// One job in flight at most (a paced host's lane).
    capped: bool,
    /// A capped lane's job is in flight.
    busy: bool,
}

impl<T> WorkQueue<T> {
    /// Lanes in the order their hosts first appear; `host` is the job's
    /// paced host, `None` for an unpaced or unknown one.
    pub(super) fn new(jobs: impl IntoIterator<Item = (T, Option<String>)>) -> Self {
        let mut queue = Self::default();
        for (job, host) in jobs {
            queue.push(job, host);
        }
        queue
    }

    /// Queues `job` at the back of its host's lane.
    pub(super) fn push(&mut self, job: T, host: Option<String>) {
        let capped = host.is_some();
        let lanes = &mut self.lanes;
        let lane = *self.by_host.entry(host).or_insert_with(|| {
            lanes.push(Lane {
                pending: VecDeque::new(),
                capped,
                busy: false,
            });
            lanes.len() - 1
        });
        if let Some(entry) = lanes.get_mut(lane) {
            entry.pending.push_back(Work { job, lane });
        }
    }

    /// The next job to start: the first lane, from this turn on, with a
    /// job waiting and room in flight. `None` when every waiting job's
    /// host already has one in flight, or no job is left.
    pub(super) fn take(&mut self) -> Option<Work<T>> {
        let count = self.lanes.len();
        for step in 0..count {
            let at = (self.turn + step) % count;
            let Some(lane) = self.lanes.get_mut(at) else {
                continue;
            };
            if lane.busy {
                continue;
            }
            if let Some(work) = lane.pending.pop_front() {
                lane.busy = lane.capped;
                self.turn = (at + 1) % count;
                return Some(work);
            }
        }
        None
    }

    /// A job of `lane` finished: its host may take the next one.
    pub(super) fn release(&mut self, lane: usize) {
        if let Some(lane) = self.lanes.get_mut(lane) {
            lane.busy = false;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn jobs(works: &[Work<u32>]) -> Vec<u32> {
        works.iter().map(|work| work.job).collect()
    }

    /// One paced host's long run of jobs does not crowd the others out:
    /// the hosts take turns, a paced host never has two jobs out at once,
    /// and the unpaced lane has no such cap.
    #[test]
    fn the_work_queue_takes_turns_between_hosts() {
        let host = |name: &str| Some(name.to_owned());
        let mut queue = WorkQueue::new([
            (0, host("reddit.com")),
            (1, host("reddit.com")),
            (2, host("reddit.com")),
            (3, host("a.example")),
            (4, None),
            (5, None),
        ]);
        queue.push(6, host("b.example"));
        let mut started = Vec::new();
        while let Some(work) = queue.take() {
            started.push(work);
        }
        // One per paced host, every unpaced job, lanes in turn.
        assert_eq!(jobs(&started), [0, 3, 4, 6, 5]);
        assert!(queue.take().is_none(), "reddit.com is busy");

        let reddit = started.first().unwrap().lane;
        queue.release(reddit);
        assert_eq!(queue.take().unwrap().job, 1);
        assert!(queue.take().is_none(), "reddit.com is busy again");
        // A job queued later joins the back of its host's lane.
        queue.push(7, host("reddit.com"));
        queue.release(reddit);
        assert_eq!(queue.take().unwrap().job, 2);
        queue.release(reddit);
        assert_eq!(queue.take().unwrap().job, 7);
        queue.release(reddit);
        assert!(queue.take().is_none(), "every job has started");
        // Releasing a lane that does not exist is harmless.
        queue.release(99);
        assert!(WorkQueue::<u32>::new([]).take().is_none());
    }
}
