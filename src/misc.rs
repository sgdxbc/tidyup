use std::{
    sync::atomic::{AtomicU16, AtomicUsize, Ordering},
    thread::available_parallelism,
};

use nix::{
    sched::{sched_setaffinity, CpuSet},
    unistd::Pid,
};

static CORE_ID: AtomicUsize = AtomicUsize::new(0);

pub fn bind_core() -> usize {
    if option_env!("CI") == Some("true") {
        return 0;
    }

    let id = CORE_ID.fetch_add(1, Ordering::SeqCst);
    let mut cpu_set = CpuSet::new();
    cpu_set.set(id % available_parallelism().unwrap()).unwrap();
    sched_setaffinity(Pid::from_raw(0), &cpu_set).unwrap();
    id
}

// FIXME ensure unique id when spawn clients on multiple machines
pub fn alloc_client_id() -> u16 {
    static ID: AtomicU16 = AtomicU16::new(1);
    ID.fetch_add(1, Ordering::SeqCst)
}
