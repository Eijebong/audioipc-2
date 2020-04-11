// Copyright © 2017 Mozilla Foundation
//
// This program is made available under an ISC-style license.  See the
// accompanying file LICENSE for details

use crate::rpc::Handler;
use std::task::{Context, Poll};
use futures::{Future, Sink, Stream};
use std::pin::Pin;
use std::fmt;
use std::io;

pub struct Driver<T>
where
    T: Handler + Unpin,
{
    // Glue
    handler: T,

    // True as long as the connection has more request frames to read.
    run: bool,

    // True when the transport is fully flushed
    is_flushed: bool,
}

impl<T> Driver<T>
where
    T: Handler + Unpin,
{
    /// Create a new rpc driver with the given service and transport.
    pub fn new(handler: T) -> Driver<T> {
        Driver {
            handler,
            run: true,
            is_flushed: true,
        }
    }

    /// Returns true if the driver has nothing left to do
    fn is_done(&self) -> bool {
        !self.run && self.is_flushed && !self.has_in_flight()
    }

    /// Process incoming messages off the transport.
    fn receive_incoming(&mut self, cx: &mut Context) -> io::Result<()> {
        while self.run {
            if let Poll::Ready(req) = self.handler.transport().poll_next(cx)? {
                self.process_incoming(req)?;
            } else {
                break;
            }
        }
        Ok(())
    }

    /// Process an incoming message
    fn process_incoming(&mut self, req: Option<T::In>) -> io::Result<()> {
        trace!("process_incoming");
        // At this point, the service & transport are ready to process the
        // request, no matter what it is.
        match req {
            Some(message) => {
                trace!("received message");

                if let Err(e) = self.handler.consume(message) {
                    // TODO: Should handler be infalliable?
                    panic!("unimplemented error handling: {:?}", e);
                }
            }
            None => {
                trace!("received None");
                // At this point, we just return. This works
                // because poll with be called again and go
                // through the receive-cycle again.
                self.run = false;
            }
        }

        Ok(())
    }

    /// Send outgoing messages to the transport.
    fn send_outgoing(&mut self, cx: &mut Context) -> io::Result<()> {
        trace!("send_responses");
        loop {
            match self.handler.produce(cx)? {
                Poll::Ready(Some(message)) => {
                    trace!("  --> got message");
                    self.process_outgoing(message, cx)?;
                }
                Poll::Ready(None) => {
                    trace!("  --> got None");
                    // The service is done with the connection.
                    self.run = false;
                    break;
                }
                // Nothing to dispatch
                Poll::Pending => {
                    cx.waker().wake_by_ref();
                    break
                }
            }
        }

        Ok(())
    }

    fn process_outgoing(&mut self, message: T::Out, cx: &mut Context) -> io::Result<()> {
        trace!("process_outgoing");
        self.handler.transport().poll_ready(cx)?;
        assert_send(self.handler.transport(), message)?;

        Ok(())
    }

    fn flush(&mut self, cx: &mut Context) -> io::Result<()> {
        self.is_flushed = self.handler.transport().poll_flush(cx)?.is_ready();

        // TODO:
        Ok(())
    }

    fn has_in_flight(&self) -> bool {
        self.handler.has_in_flight()
    }
}

impl<T> Future for Driver<T>
where
    T: Handler + Unpin,
{
    type Output = Result<(), io::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        trace!("rpc::Driver::tick");

        // First read off data from the socket
        self.receive_incoming(cx)?;

        // Handle completed responses
        self.send_outgoing(cx)?;

        // Try flushing buffered writes
        self.flush(cx)?;

        if self.is_done() {
            trace!("  --> is done.");
            return Poll::Ready(Ok(()));
        }

        // Tick again later
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

fn assert_send<Item, S: Sink<Item>>(s: Pin<&mut S>, item: Item) -> Result<(), S::Error> {
    match s.start_send(item) {
        Ok(()) => Ok(()),
        Err(_) => panic!(
            "sink reported itself as ready after `poll_ready` but was \
             then unable to accept a message"
        ),
    }
}

impl<T> fmt::Debug for Driver<T>
where
    T: Handler + fmt::Debug + Unpin,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("rpc::Handler")
            .field("handler", &self.handler)
            .field("run", &self.run)
            .field("is_flushed", &self.is_flushed)
            .finish()
    }
}
