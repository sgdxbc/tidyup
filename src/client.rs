use std::{
    borrow::{Borrow, BorrowMut},
    net::IpAddr,
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
        R: AsMut<Transport<R>>,
    {
        runtime.create_transport((addr.into(), 0).into(), ReplicaId::MAX, move |container| {
            self_mut(container).borrow_mut()
        })
    }
}

pub struct Null<T> {
    receiver: T,
    //
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
    pub fn new(receiver: T) -> Self {
        Self { receiver }
    }
}

impl<T> TransportReceiver for Null<T>
where
    T: TransportReceiver + Client,
{
    fn receive_message(&mut self, message: &[u8]) {
        self.receiver.receive_message(message);
        if let Some(result) = self.receiver.take_result() {
            assert_eq!(&*result, &[]);
            //
            self.receiver.invoke(Box::new([]));
        }
    }
}

impl<T> LoopClient<T> for Null<T> where T: TransportReceiver + Client + 'static {}
