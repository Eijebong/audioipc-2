// This is a derived version of simple/pipeline/server.rs from
// tokio_proto crate used under MIT license.
//
// Original version of server.rs:
// https://github.com/tokio-rs/tokio-proto/commit/8fb8e482dcd55cf02ceee165f8e08eee799c96d3
//
// The following modifications were made:
// * Simplify the code to implement RPC for pipeline requests that
//   contain simple request/response messages:
//   * Remove `Error` types,
//   * Remove `bind_transport` fn & `BindTransport` type,
//   * Remove all "Lift"ing functionality.
//   * Remove `Service` trait since audioipc doesn't use `tokio_service`
//     crate.
//
// Copyright (c) 2016 Tokio contributors
//
// Permission is hereby granted, free of charge, to any
// person obtaining a copy of this software and associated
// documentation files (the "Software"), to deal in the
// Software without restriction, including without
// limitation the rights to use, copy, modify, merge,
// publish, distribute, sublicense, and/or sell copies of
// the Software, and to permit persons to whom the Software
// is furnished to do so, subject to the following
// conditions:
//
// The above copyright notice and this permission notice
// shall be included in all copies or substantial portions
// of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF
// ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED
// TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A
// PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT
// SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
// CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION
// OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR
// IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
// DEALINGS IN THE SOFTWARE.

use crate::rpc::driver::Driver;
use crate::rpc::Handler;
use futures::{Future, Sink, Stream};
use core::task::{Context, Poll};
use std::collections::VecDeque;
use std::io;
use futures_util::sink::SinkExt;


use std::pin::Pin;
/// Bind an async I/O object `io` to the `server`.
pub fn bind_server<S>(transport: S::Transport, server: S) -> impl Future<Output=Result<(), std::io::Error>>
where
    S: Server + Unpin,
    S::Response: Unpin,
    S::Transport: Unpin,
    S::Future: Unpin
{
    let fut = {
        let handler = ServerHandler {
            server,
            transport,
            in_flight: VecDeque::with_capacity(32),
        };
        Driver::new(handler)
    };

    fut
}

pub trait Server: 'static {
    /// Request
    type Request: 'static;

    /// Response
    type Response: 'static;

    /// Future
    type Future: Future<Output = Result<Self::Response, ()>> + Unpin;

    /// The message transport, which works with async I/O objects of
    /// type `A`.
    type Transport: 'static
        + Stream<Item = Result<Self::Request, io::Error>>
        + Sink<Self::Response, Error = io::Error>
        + Unpin;

    /// Process the request and return the response asynchronously.
    fn process(&mut self, req: Self::Request) -> Self::Future;
}

////////////////////////////////////////////////////////////////////////////////

struct ServerHandler<S>
where
    S: Server + Unpin,
{
    // The service handling the connection
    server: S,
    // The transport responsible for sending/receving messages over the wire
    transport: S::Transport,
    // FIFO of "in flight" responses to requests.
    in_flight: VecDeque<InFlight<S::Response, S::Future>>,
}

impl<S> Handler for ServerHandler<S>
where
    S: Server + Unpin,
{
    type In = S::Request;
    type Out = S::Response;
    type Transport = S::Transport;

    /// Mutable reference to the transport
    fn transport(&mut self) -> Pin<&mut Self::Transport> {
        // TODO: Dfinitely wrong
        Pin::new(&mut self.transport)
    }

    /// Consume a message
    fn consume(&mut self, request: Self::In) -> io::Result<()> {
        trace!("ServerHandler::consume");
        let response = self.server.process(request);
        self.in_flight.push_back(InFlight::Active(response));

        // TODO: Should the error be handled differently?
        Ok(())
    }

    /// Produce a message
    fn produce(&mut self, cx: &mut Context) -> Poll<Result<Option<Self::Out>, io::Error>> {
        trace!("ServerHandler::produce");

        // Make progress on pending responses
        for pending in &mut self.in_flight {
            pending.poll(cx);
        }

        // Is the head of the queue ready?
        match self.in_flight.front() {
            Some(&InFlight::Done(_)) => {}
            _ => {
                trace!("  --> not ready");
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
        }

        // Return the ready response
        match self.in_flight.pop_front() {
            Some(InFlight::Done(Ok(res))) => {
                trace!("  --> received response");
                Poll::Ready(Ok(Some(res)))
            }
            _ => panic!(),
        }
    }

    /// RPC currently in flight
    fn has_in_flight(&self) -> bool {
        !self.in_flight.is_empty()
    }
}

impl<S: Server + Unpin> Drop for ServerHandler<S> {
    fn drop(&mut self) {
        let _ = self.transport.close();
        self.in_flight.clear();
    }
}

////////////////////////////////////////////////////////////////////////////////

enum InFlight<R, F: Future<Output=Result<R, ()>> + Unpin> {
    Active(F),
    Done(F::Output),
}

impl<R, F: Future<Output=Result<R, ()>> + Unpin> InFlight<R, F> {
    fn poll(&mut self, cx: &mut Context) {
        let res = match *self {
            InFlight::Active(ref mut f) => match std::pin::Pin::new(f).poll(cx) {
                Poll::Ready(Ok(e)) => e,
                Poll::Ready(Err(_)) => unreachable!(),
                Poll::Pending => {
                    cx.waker().wake_by_ref();
                    return
                }
            },
            _ => return,
        };
        *self = InFlight::Done(Ok(res));
    }
}
