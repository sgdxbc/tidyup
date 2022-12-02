use std::{
    collections::BTreeSet,
    net::{IpAddr, UdpSocket},
    num::NonZeroUsize,
    os::unix::io::{AsRawFd, RawFd},
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use nix::sys::epoll::{epoll_create, epoll_ctl, epoll_wait, EpollEvent, EpollFlags, EpollOp};

use crate::{
    core::{ClientCommon, Clock, RxChannel, TransportConfig, TxChannel},
    misc::{alloc_client_id, bind_core},
    unreplicated, ClientState, OptionInstant,
};

pub struct Program<T> {
    clients: Box<[T]>,
    epoll_fd: RawFd,
    // client => instant and (instant, client) timer tables
    poll_instants: Box<[OptionInstant]>,
    instant_polls: BTreeSet<(OptionInstant, usize)>,

    run_duration: Duration,
    n_result: Arc<AtomicU32>,
}

impl Program<unreplicated::Client> {
    pub fn new(
        n_client: NonZeroUsize,
        config: Arc<TransportConfig>,
        ip: IpAddr,
        run_duration: Duration,
        n_result: Arc<AtomicU32>,
    ) -> Self {
        let epoll_fd = epoll_create().unwrap();
        let clients = (0..n_client.get())
            .map(|i| {
                let socket = UdpSocket::bind((ip, 0)).unwrap();
                socket.set_nonblocking(true).unwrap();
                epoll_ctl(
                    epoll_fd,
                    EpollOp::EpollCtlAdd,
                    socket.as_raw_fd(),
                    &mut EpollEvent::new(EpollFlags::EPOLLIN | EpollFlags::EPOLLET, i as _),
                )
                .unwrap();
                let common = ClientCommon {
                    id: alloc_client_id(),
                    config: config.clone(),
                    tx: TxChannel::Udp(socket.try_clone().unwrap()),
                    rx_addr: socket.local_addr().unwrap(),
                    rx: RxChannel::Udp(socket),
                    clock: Clock::Real,
                };
                unreplicated::Client::new(common)
            })
            .collect();
        let poll_instants = vec![Default::default(); n_client.get()].into_boxed_slice();
        Self {
            clients,
            epoll_fd,
            poll_instants,
            instant_polls: (0..n_client.get())
                .map(|i| (Default::default(), i))
                .collect(),
            run_duration,
            n_result,
        }
    }
}

impl<T> Program<T> {
    pub fn run(&mut self)
    where
        T: ClientState,
    {
        bind_core();

        for client in &mut self.clients[..] {
            client.invoke(Box::new([])); //
        }
        for i in 0..self.clients.len() {
            while self.poll_client(i) {}
            self.update_timer(i);
        }

        let start = Instant::now();
        let mut now;
        let mut event_buffer = [EpollEvent::empty(); 64];
        let mut events = &[][..];
        while {
            now = Instant::now();
            now - start < self.run_duration
        } {
            let &(poll_at, i) = self.instant_polls.iter().next().unwrap();
            if poll_at < Default::default() {
                while self.poll_client(i) {} // is this possible to live lock?
                self.update_timer(i);
            }

            if events.is_empty() {
                let len = epoll_wait(self.epoll_fd, &mut event_buffer, 0).unwrap();
                events = &event_buffer[..len];
            }
            if let Some(event) = events.first() {
                let i = event.data() as usize;
                if !self.poll_client(i) {
                    self.update_timer(i);
                    events = &events[1..];
                }
            }
        }
    }

    fn poll_client(&mut self, i: usize) -> bool
    where
        T: ClientState,
    {
        let poll_again = self.clients[i].poll();
        if let Some(_result) = self.clients[i].take_result() {
            // check result
            self.n_result.fetch_add(1, Ordering::SeqCst);
            self.clients[i].invoke(Box::new([])); //
        }
        poll_again
    }

    fn update_timer(&mut self, i: usize)
    where
        T: ClientState,
    {
        let poll_at = self.clients[i].poll_at();
        if poll_at == self.poll_instants[i] {
            return;
        }
        let removed = self.instant_polls.remove(&(self.poll_instants[i], i));
        assert!(removed);
        self.instant_polls.insert((poll_at, i));
        self.poll_instants[i] = poll_at;
    }
}
