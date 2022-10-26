use std::{
    borrow::{Borrow, BorrowMut},
    net::IpAddr,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use messages::ReplicaId;

use crate::transport::{Transport, TransportReceiver, TransportRuntime};

pub trait Client {
    fn invoke(&mut self, op: Box<[u8]>);
    fn take_result(&mut self) -> Option<Box<[u8]>>;
}

pub trait LoopClient<T>
where
    Self: TransportReceiver + BorrowMut<T> + 'static,
{
    fn create_transport<M, R>(
        runtime: &mut TransportRuntime<M>,
        addr: impl Into<IpAddr>,
        self_mut: impl Fn(&mut M) -> &mut Self + 'static + Clone,
    ) -> Transport<R>
    where
        T: TransportReceiver + BorrowMut<R>,
        R: AsMut<Transport<R>> + Client,
    {
        let mut transport =
            runtime.create_transport((addr.into(), 0).into(), ReplicaId::MAX, move |container| {
                self_mut(container).borrow_mut()
            });
        runtime.create_timeout(&mut transport, Duration::ZERO, |receiver| {
            receiver.invoke(Box::new([]))
        });
        transport
    }
}

pub struct Null<T> {
    receiver: T,
    n_complete: Arc<AtomicU32>,
    invoke_instant: Instant,
    pub latencies: Vec<Duration>,
}

impl<T> Borrow<T> for Null<T> {
    fn borrow(&self) -> &T {
        &self.receiver
    }
}

impl<T> BorrowMut<T> for Null<T> {
    fn borrow_mut(&mut self) -> &mut T {
        &mut self.receiver
    }
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
}

impl<T> TransportReceiver for Null<T>
where
    T: TransportReceiver + Client,
{
    fn receive_message(&mut self, message: &[u8]) {
        self.receiver.receive_message(message);
        if let Some(result) = self.receiver.take_result() {
            println!("get result");
            self.latencies.push(Instant::now() - self.invoke_instant);
            assert_eq!(&*result, &[]);
            self.n_complete.fetch_add(1, Ordering::SeqCst);

            self.invoke_instant = Instant::now();
            self.receiver.invoke(Box::new([]));
        }
    }
}

impl<T> LoopClient<T> for Null<T> where T: TransportReceiver + Client + 'static {}
