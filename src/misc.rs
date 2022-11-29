use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};

use nix::{
    sched::{sched_setaffinity, CpuSet},
    unistd::Pid,
};

static CORE_ID: AtomicUsize = AtomicUsize::new(0);

pub fn bind_core() -> usize {
    let id = CORE_ID.fetch_add(1, Ordering::SeqCst);
    let mut cpu_set = CpuSet::new();
    cpu_set.set(id).unwrap();
    sched_setaffinity(Pid::from_raw(0), &cpu_set).unwrap();
    id
}

pub fn alloc_client_id() -> u16 {
    static ID: AtomicU16 = AtomicU16::new(1);
    ID.fetch_add(1, Ordering::SeqCst)
}
