use std::{
    mem::{replace, take},
    thread::{spawn, JoinHandle},
};

use crossbeam_channel::TryRecvError;

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
pub enum EffectRunner<T> {
    ThreadPool {
        handles: Box<[JoinHandle<()>]>,
        sender: crossbeam_channel::Sender<Box<dyn FnOnce(&mut T) + Send>>,
    },
    Inline(T),
}

pub trait EffectContext {
    fn idle_poll(&mut self) -> bool {
        false
    }
}

impl<T> EffectRunner<T> {
    pub fn new(context_iter: impl ExactSizeIterator<Item = T>) -> Self
    where
        T: EffectContext + Send + 'static,
    {
        assert_ne!(context_iter.len(), 0);
        assert!(context_iter.len() < u64::BITS as _);

        let (sender, receiver) = crossbeam_channel::bounded(1024);
        Self::ThreadPool {
            handles: context_iter
                .enumerate()
                .map(|(i, context)| {
                    let receiver = receiver.clone();
                    spawn(move || Self::worker_loop(i, context, receiver))
                })
                .collect::<Vec<_>>()
                .into(),
            sender,
        }
    }

    fn worker_loop(
        _i: usize,
        mut context: T,
        receiver: crossbeam_channel::Receiver<Box<dyn FnOnce(&mut T) + Send>>,
    ) where
        T: EffectContext,
    {
        bind_core();

        loop {
            match receiver.try_recv() {
                Ok(effect) => effect(&mut context),
                Err(TryRecvError::Empty) => {
                    context.idle_poll();
                }
                Err(TryRecvError::Disconnected) => return,
            }
        }
    }

    pub fn run(&mut self, effect: impl FnOnce(&mut T) + Send + 'static) -> u32 {
        match self {
            Self::Inline(context) => {
                effect(context);
                0
            }
            Self::ThreadPool { sender, .. } => {
                sender.send(Box::new(effect)).unwrap();
                0
            }
        }
    }

    pub fn run_idle(&mut self) -> bool
    where
        T: EffectContext,
    {
        if let Self::Inline(context) = self {
            context.idle_poll()
        } else {
            false
        }
    }
}

impl<T> Drop for EffectRunner<T> {
    fn drop(&mut self) {
        let Self::ThreadPool {  handles , sender, ..} = self else {
            return;
        };
        let (temp_sender, _) = crossbeam_channel::bounded(0);
        drop(replace(sender, temp_sender));
        for handle in Vec::from(take(handles)).into_iter() {
            handle.join().unwrap()
        }
    }
}
