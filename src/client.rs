use std::{
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use crate::transport::{ReactorMut, TransportReceiver};

pub trait Client {
    fn invoke(&mut self, op: Box<[u8]>);
    fn take_result(&mut self) -> Option<Box<[u8]>>;
}

pub struct Null<T> {
    receiver: T,
    n_complete: Arc<AtomicU32>,
    invoke_instant: Instant,
    pub latencies: Vec<Duration>,
}

impl<T> Null<T> {
    pub fn new(receiver: T, n_complete: Arc<AtomicU32>) -> Self {
        Self {
            receiver,
            n_complete,
            invoke_instant: Instant::now(),
            latencies: Vec::new(),
        }
    }

    pub fn initiate(&mut self)
    where
        T: Client,
    {
        self.receiver.invoke(Box::new([]));
    }
}

impl<T, R> ReactorMut<R> for Null<T>
where
    T: ReactorMut<R>,
{
    fn reactor_mut(&mut self) -> &mut R {
        self.receiver.reactor_mut()
    }
}

impl<T> TransportReceiver for Null<T>
where
    T: TransportReceiver + Client,
{
    fn receive_message(&mut self, message: &[u8]) {
        self.receiver.receive_message(message);
        if let Some(result) = self.receiver.take_result() {
            self.latencies.push(Instant::now() - self.invoke_instant);
            assert_eq!(&*result, &[]);
            self.n_complete.fetch_add(1, Ordering::SeqCst);

            self.invoke_instant = Instant::now();
            self.receiver.invoke(Box::new([]));
        }
    }
}
