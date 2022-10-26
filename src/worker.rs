use std::{
    mem::{replace, take},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread::{spawn, JoinHandle},
};

use nix::{
    sched::{sched_setaffinity, CpuSet},
    unistd::Pid,
};

use crate::transport::Worker;

pub enum Work {
    Inline(Worker),
    Pool(WorkerPool),
}

pub struct WorkerPool {
    handles: Vec<JoinHandle<()>>,
    // 1 if worker is ready, i.e. slot can be `try_lock`ed by master
    ready_bits: Arc<AtomicU64>,
    tasks: Vec<Arc<Mutex<Box<dyn FnOnce(&mut Worker) + Send>>>>,
}

impl WorkerPool {
    fn run(
        mut worker: Worker,
        id: usize,
        ready_bits: Arc<AtomicU64>,
        task: Arc<Mutex<Box<dyn FnOnce(&mut Worker) + Send>>>,
    ) {
        let mut cpu_set = CpuSet::new();
        cpu_set.set(id + 1).unwrap(); // save first core for main thread
        sched_setaffinity(Pid::from_raw(0), &cpu_set).unwrap();

        let mask = 1 << id;
        loop {
            let mut snapshot_bits;
            while {
                snapshot_bits = ready_bits.load(Ordering::SeqCst);
                snapshot_bits & mask != 0
            } {}
            if snapshot_bits == 0 {
                return;
            }

            let task = { replace(&mut *task.try_lock().unwrap(), Box::new(|_| unreachable!())) };
            task(&mut worker);

            while let Err(bits) = ready_bits.compare_exchange_weak(
                snapshot_bits,
                snapshot_bits | mask,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                snapshot_bits = bits;
            }
        }
    }

    pub fn new(n_worker: usize, worker: Worker) -> Self {
        let running_bit = 1 << (u64::BITS - 1);
        let ready_bits = Arc::new(AtomicU64::new(running_bit | ((1 << n_worker) - 1)));
        let mut tasks = Vec::new();
        let handles = (0..n_worker)
            .map(|id| {
                let worker = worker.clone();
                let ready_bits = ready_bits.clone();
                let task = Arc::new(Mutex::new(Box::new(|_: &mut _| unreachable!()) as _));
                tasks.push(task.clone());
                spawn(move || Self::run(worker, id, ready_bits, task))
            })
            .collect::<Vec<_>>();
        Self {
            handles,
            ready_bits,
            tasks,
        }
    }

    pub fn work(&mut self, task: impl FnOnce(&mut Worker) + Send + 'static) {
        let mut snapshot_bits;
        let mut id;
        while {
            snapshot_bits = self.ready_bits.load(Ordering::SeqCst);
            id = snapshot_bits.trailing_zeros();
            id == u64::BITS - 1
        } {}

        *self.tasks[id as usize].try_lock().unwrap() = Box::new(task);

        let mask = 1 << id;
        while let Err(bits) = self.ready_bits.compare_exchange_weak(
            snapshot_bits,
            snapshot_bits ^ mask,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            snapshot_bits = bits;
        }
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        self.ready_bits.store(0, Ordering::SeqCst);
        for handle in take(&mut self.handles) {
            handle.join().unwrap();
        }
    }
}
