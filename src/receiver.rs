use std::time::Instant;

pub trait Receiver {
    fn poll(&mut self) -> bool;
    fn poll_at(&self) -> Option<Instant>;
}

pub trait Client
where
    Self: Receiver,
{
    fn invoke(&mut self, op: Box<[u8]>);
    fn take_result(&mut self) -> Option<Box<[u8]>>;
}
