// Copyright 2020 Parity Technologies (UK) Ltd.
//
// Permission is hereby granted, free of charge, to any person obtaining a
// copy of this software and associated documentation files (the "Software"),
// to deal in the Software without restriction, including without limitation
// the rights to use, copy, modify, merge, publish, distribute, sublicense,
// and/or sell copies of the Software, and to permit persons to whom the
// Software is furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
// OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
// FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
// DEALINGS IN THE SOFTWARE.

use std::{
    sync::{
        mpsc::{Receiver, RecvError, SyncSender},
        Arc, Mutex,
    },
    time::Duration,
};

use zeroconf::{MdnsService, ServiceRegistration, TxtRecord};
// Necessary traits:
use zeroconf::prelude::*;

use libp2p_core::{multiaddr::Protocol, Multiaddr, PeerId};

use crate::error::Error;
use crate::Result;

/// Data for a worker thread communicating with Avahi/Bonjour.
pub struct Worker {
    service: MdnsService,
}

/// Messages going to this worker thread.
#[derive(Debug)]
pub enum ToWorker {
    /// (Re-)initialize the service.
    ReInit {
        peer_id: PeerId,
        addrs: Vec<Multiaddr>,
    },
    /// Quit this worker.
    ///
    /// The `run` function will return to its caller upon reception of this message.
    Quit,
}

/// Messages sent from this worker thread.
pub enum FromWorker {
    /// Service registration has finished with the given `Result`.
    ServiceRegistered(zeroconf::Result<ServiceRegistration>),
    /// Thread quit due to some error.
    QuitWithError(Error),
}

impl Worker {
    /// Created mdns service registration.
    ///
    /// It will make sure `service` is populated and up2date with our current listening addresses.
    pub fn new(
        peer_id: &PeerId,
        addrs: Vec<Multiaddr>,
        sender: &SyncSender<FromWorker>,
    ) -> Result<Worker> {
        let ports = addrs.clone().into_iter().filter_map(get_tcp_udp_port);
        let port = ports
            .next()
            .expect("Passed Multiaddr should contain a port number.");
        // Make sure we don't have disagreeing other ports:
        debug_assert_eq!(ports.filter(|p| *p != port).next(), None);

        // Always "udp" based on:
        // https://github.com/libp2p/specs/blob/55b463e2ff8231491ec35c56432331a783e8786a/discovery/mdns.md
        let mut service = MdnsService::new("_p2p._udp.local", port.get_port());
        let mut txt_record = TxtRecord::new();
        let context: Arc<Mutex<()>> = Arc::default();

        for addr in addrs {
            txt_record
                .insert("dnsaddr", &addr.to_string())
                .map_err(Error::SetTxtRecordFailed)?;
        }

        let callback_send = sender.clone();
        service.set_registered_callback(Box::new(|r, _| {
            callback_send.send(FromWorker::ServiceRegistered(r));
        }));
        service.set_context(Box::new(context));
        service.set_txt_record(txt_record);
        Ok(Worker { service })
    }

    /// Run the worker.
    ///
    /// This function will usually be spawned in a thread (thread::spawn).
    ///
    /// # Panics
    ///
    /// The first message sent to this thread must be `ReInit`, this function panics if any other
    /// message is sent first.
    pub fn run(sender: SyncSender<FromWorker>, receiver: Receiver<ToWorker>) {
        match Worker::run_inner(&sender, receiver) {
            Ok(()) => return,
            Err(err) => sender
                .send(FromWorker::QuitWithError(err))
                .expect("Main thread should still be alive, I would like to report my errors."),
        }
    }

    fn run_inner(sender: &SyncSender<FromWorker>, receiver: Receiver<ToWorker>) -> Result<()> {
        let mut w = match receiver.recv() {
            Ok(ToWorker::ReInit { peer_id, addrs })
                => Worker::new(&peer_id, addrs, sender)?,
            Err(RecvError)
                => return Ok(()),
            Ok(r)
                => panic!("You have to initialize the service (ReInit message) prior to using it, thus we are not handling request: {:#?}", r),
        };

        let event_loop = w.service.register().map_err(Error::RegisterServiceFailed)?;

        loop {
            match receiver.try_recv() {
                Ok(ToWorker::ReInit { peer_id, addrs }) => {
                    w = Worker::new(&peer_id, addrs, sender)?
                }
                Ok(ToWorker::Quit) => break,
                Err(std::sync::mpsc::TryRecvError::Empty) => (),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }

            event_loop.poll(Duration::from_secs(1)).unwrap();
        }
        Ok(())
    }
}

/// Tcp or Udp port.
#[derive(Debug, Eq, PartialEq)]
enum Port {
    Tcp(u16),
    Udp(u16),
}

impl Port {
    fn get_port(&self) -> u16 {
        match self {
            Port::Tcp(p) => *p,
            Port::Udp(p) => *p,
        }
    }
}

/// Get the port out of a `Multiaddr` (if any).
fn get_tcp_udp_port(addr: Multiaddr) -> Option<Port> {
    addr.iter()
        .filter_map(|p| match p {
            Protocol::Tcp(port) => Some(Port::Tcp(port)),
            Protocol::Udp(port) => Some(Port::Udp(port)),
            _ => None,
        })
        .next()
}
