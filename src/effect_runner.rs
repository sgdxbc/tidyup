use std::{
    iter::{once, repeat_with},
    mem::{replace, take},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread::{spawn, JoinHandle},
};

use crate::misc::bind_core;

/// A thread pool utility for general propose.
///
/// Originally, this struct is for helping single-thread application to make
/// "effects" to the outer world, e.g., send packets, more efficiently than
/// performing it on local thread. The effects usually take long run time
/// (because of heavy computation or trapping in kernel), do not need to access
/// application state so it can perfectly execute concurrently to the
/// application. Occasionally there's control dependency between application and
/// effects, for example application wants to backup sent messages after the
/// sending is finished. In such case a back channel can be used for signaling,
/// along with some external mechanism to proactively wake up application.
///
/// Compare to a normal thread pool, like the one provided by Rayon, this pool
/// explicitly passes a context object typed `T` to every worker thread, which
/// can be leveraged by `FnOnce(&mut T)` effects. This can save some effort and
/// performance penalty of cloning and capturing this object for every effects.
/// It can also be used to "inject" a context for application when someone else
/// is providing the runner for the application. Also, this pool proactively
/// schedules effects from application thread, with strong back pressure that
/// blocks application immediately when no idle worker is for an effect, instead
/// of setting up a queue and work-stealing workers. According to previous
/// exprience this choice is better for this project's use case.
pub struct EffectRunner<T> {
    idle_bits: Arc<AtomicU64>,
    slots: Arc<[Mutex<Box<dyn FnOnce(&mut T) + Send>>]>,
    handles: Box<[JoinHandle<()>]>,
}

pub trait EffectContext {
    fn idle_poll(&mut self) -> bool {
        false
    }
}

impl<T> EffectRunner<T> {
    const SHUTDOWN: u64 = 0;

    pub fn new(context_iter: impl ExactSizeIterator<Item = T>) -> Self
    where
        T: EffectContext + Send + 'static,
    {
        assert!(context_iter.len() < u64::BITS as _);
        let idle_bits = Arc::new(AtomicU64::new(
            (0..context_iter.len() as u32)
                // the always-set bit before shutdown
                .chain(once(u64::BITS - 1))
                .map(|i| 1 << i)
                .sum(),
        ));
        let slots = Arc::from(
            repeat_with(|| Mutex::new(Box::new(|_: &mut _| unreachable!()) as _))
                .take(context_iter.len())
                .collect::<Vec<_>>(),
        );
        Self {
            handles: context_iter
                .enumerate()
                .map(|(i, context)| {
                    let idle_bits = Arc::clone(&idle_bits);
                    let slots = Arc::clone(&slots);
                    spawn(move || Self::worker_loop(i, context, idle_bits, slots))
                })
                .collect::<Vec<_>>()
                .into(),
            slots,
            idle_bits,
        }
    }

    fn worker_loop(
        i: usize,
        mut context: T,
        idle_bits: Arc<AtomicU64>,
        slots: Arc<[Mutex<Box<dyn FnOnce(&mut T) + Send>>]>,
    ) where
        T: EffectContext,
    {
        bind_core();

        let mut bits;
        while {
            while {
                bits = idle_bits.load(Ordering::SeqCst);
                bits & (1 << i) != 0
            } {
                context.idle_poll();
            }
            bits != Self::SHUTDOWN
        } {
            replace(
                &mut *slots[i].try_lock().unwrap(),
                Box::new(|_| unreachable!()),
            )(&mut context);
            while let Err(bits_) = idle_bits.compare_exchange_weak(
                bits,
                bits | (1 << i),
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                bits = bits_;
            }
        }
    }

    pub fn run(&self, effect: impl FnOnce(&mut T) + Send + 'static) -> u32 {
        let (mut bits, mut i);
        while {
            bits = self.idle_bits.load(Ordering::SeqCst);
            i = bits.trailing_zeros();
            i == u64::BITS - 1
        } {}
        *self.slots[i as usize].try_lock().unwrap() = Box::new(effect);
        while let Err(bits_) = self.idle_bits.compare_exchange_weak(
            bits,
            bits ^ (1 << i),
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            bits = bits_
        }
        i
    }
}

impl<T> Drop for EffectRunner<T> {
    fn drop(&mut self) {
        self.idle_bits.store(Self::SHUTDOWN, Ordering::SeqCst);
        for handle in Vec::from(take(&mut self.handles)).into_iter() {
            handle.join().unwrap()
        }
    }
}
