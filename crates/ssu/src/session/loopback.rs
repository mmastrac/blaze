use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use crate::session::{SessionEndpoint, SessionRecvEndpoint, SessionSendEndpoint, Ticked};

pub struct LoopbackSession {
    queue: Arc<Mutex<VecDeque<u8>>>,
}

pub struct LoopbackRecvSession {
    queue: Arc<Mutex<VecDeque<u8>>>,
}

pub struct LoopbackSendSession {
    queue: Arc<Mutex<VecDeque<u8>>>,
}

impl LoopbackSession {
    pub fn new(initial: String) -> Self {
        LoopbackSession {
            queue: Arc::new(Mutex::new(VecDeque::from_iter(initial.bytes()))),
        }
    }
}

impl SessionEndpoint for LoopbackSession {
    fn recv(&mut self) -> Ticked {
        match self.queue.lock().unwrap().pop_front() {
            Some(b) => Ticked::Byte(b),
            None => Ticked::IdleInput,
        }
    }

    fn send(&mut self, b: u8) {
        self.queue.lock().unwrap().push_back(b);
    }

    fn split(
        self: Box<Self>,
    ) -> (
        Box<dyn SessionRecvEndpoint + Send + 'static>,
        Box<dyn SessionSendEndpoint + Send + 'static>,
    ) {
        (
            Box::new(LoopbackRecvSession {
                queue: self.queue.clone(),
            }),
            Box::new(LoopbackSendSession { queue: self.queue }),
        )
    }
}

impl SessionRecvEndpoint for LoopbackRecvSession {
    fn recv(&mut self) -> Ticked {
        match self.queue.lock().unwrap().pop_front() {
            Some(b) => Ticked::Byte(b),
            None => Ticked::IdleInput,
        }
    }
}

impl SessionSendEndpoint for LoopbackSendSession {
    fn send(&mut self, b: u8) {
        self.queue.lock().unwrap().push_back(b);
    }
}
