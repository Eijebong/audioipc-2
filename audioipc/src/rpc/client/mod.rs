// This is a derived version of simple/pipeline/client.rs from
// tokio_proto crate used under MIT license.
//
// Original version of client.rs:
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
use futures::channel::oneshot;
use std::pin::Pin;
use std::task::{Context, Poll};
use futures::{Sink, Stream};
use futures_util::{sink::SinkExt};
use std::collections::VecDeque;
use std::io;


mod proxy;

pub use self::proxy::{ClientProxy, Response};

pub fn bind_client<C>(transport: C::Transport) -> proxy::ClientProxy<C::Request, C::Response>
where
    C: Client,
    C::Transport: Unpin + Send,
    C::Response: Send,
    C::Request: Send
{
    let (tx, rx) = proxy::channel();

    let fut = {
        let handler = ClientHandler::<C> {
            transport,
            requests: rx,
            in_flight: VecDeque::with_capacity(32),
        };
        Driver::new(handler)
    };

    tokio::runtime::Handle::current().spawn(fut);
    tx
}

pub trait Client: 'static {
    /// Request
    type Request: 'static;

    /// Response
    type Response: 'static;

    /// The message transport, which works with async I/O objects of type `A`
    type Transport: 'static
        + Stream<Item = Result<Self::Response, io::Error>>
        + Sink<Self::Request, Error = io::Error>;
}

////////////////////////////////////////////////////////////////////////////////

struct ClientHandler<C>
where
    C: Client,
    C::Transport: Unpin
{
    transport: C::Transport,
    requests: proxy::Receiver<C::Request, C::Response>,
    in_flight: VecDeque<oneshot::Sender<C::Response>>,
}

impl<C> Handler for ClientHandler<C>
where
    C: Client,
    C::Transport: Unpin
{
    type In = C::Response;
    type Out = C::Request;
    type Transport = C::Transport;

    // TODO: def wrong
    fn transport(&mut self) -> Pin<&mut Self::Transport> {
        Pin::new(&mut self.transport)
    }

    fn consume(&mut self, response: Self::In) -> io::Result<()> {
        trace!("ClientHandler::consume");
        if let Some(complete) = self.in_flight.pop_front() {
            drop(complete.send(response));
        } else {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "request / response mismatch",
            ));
        }

        Ok(())
    }

    /// Produce a message
    fn produce(&mut self, cx: &mut Context) -> Poll<Result<Option<Self::Out>, io::Error>> {
        trace!("ClientHandler::produce");

        // Try to get a new request
        // TODO: Prob wrong
        match Pin::new(&mut self.requests).poll_next(cx) {
            Poll::Ready(Some((request, complete))) => {
                trace!("  --> received request");

                // Track complete handle
                self.in_flight.push_back(complete);

                Poll::Ready(Ok(Some(request)))
            }
            Poll::Ready(None) => {
                trace!("  --> client dropped");
                Poll::Ready(Ok(None))
            }
            Poll::Pending => {
                trace!("  --> not ready");
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }

    /// RPC currently in flight
    fn has_in_flight(&self) -> bool {
        !self.in_flight.is_empty()
    }
}

impl<C: Client> Drop for ClientHandler<C> where
C::Transport: Unpin
{
    fn drop(&mut self) {
        let _ = self.transport.close();
        self.in_flight.clear();
    }
}
