// Copyright © 2017 Mozilla Foundation
//
// This program is made available under an ISC-style license.  See the
// accompanying file LICENSE for details

use crate::codec::Codec;
use bytes::{Buf, Bytes, BytesMut};
use std::task::{Context, Poll};
use std::pin::Pin;
use futures::{Sink, Stream};
use std::io;
use futures::io::{AsyncRead, AsyncWrite};
use futures::io::{AsyncReadExt};
const INITIAL_CAPACITY: usize = 1024;
const BACKPRESSURE_THRESHOLD: usize = 4 * INITIAL_CAPACITY;

/// A unified `Stream` and `Sink` interface over an I/O object, using
/// the `Codec` trait to encode and decode the payload.
pub struct Framed<A: Unpin, C: Unpin> {
    io: A,
    codec: C,
    read_buf: BytesMut,
    write_buf: BytesMut,
    frame: Option<Bytes>,
    is_readable: bool,
    eof: bool,
}

impl<A, C> Framed<A, C>
where
    A: AsyncWrite + Unpin,
    C: Unpin
{
    // If there is a buffered frame, try to write it to `A`
    fn do_write(&mut self, cx: &mut Context) -> Poll<Result<(), io::Error>> {
        loop {
            if self.frame.is_none() {
                self.set_frame();
            }

            if self.frame.is_none() {
                return Poll::Ready(Ok(()));
            }

            let done = {
                let frame = self.frame.as_mut().unwrap();
                futures::ready!(Pin::new(&mut self.io).poll_write(cx, frame))?;
                !frame.has_remaining()
            };

            if done {
                self.frame = None;
            }
        }
    }

    fn set_frame(&mut self) {
        if self.write_buf.is_empty() {
            return;
        }

        debug_assert!(self.frame.is_none());

        self.frame = Some(self.write_buf.split().freeze());
    }
}

impl<A, C> Stream for Framed<A, C>
where
    A: AsyncRead + Unpin,
    C: Codec + Unpin,
{
    type Item = Result<C::Out, io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let self_ = &mut *self;
        loop {
            // Repeatedly call `decode` or `decode_eof` as long as it is
            // "readable". Readable is defined as not having returned `None`. If
            // the upstream has returned EOF, and the decoder is no longer
            // readable, it can be assumed that the decoder will never become
            // readable again, at which point the stream is terminated.
            if self_.is_readable {
                if self_.eof {
                    let frame = self_.codec.decode_eof(&mut self_.read_buf)?;
                    return Poll::Ready(Some(Ok(frame)));
                }

                trace!("attempting to decode a frame");

                if let Some(frame) = self_.codec.decode(&mut self_.read_buf)? {
                    trace!("frame decoded from buffer");
                    return Poll::Ready(Some(Ok(frame)));
                }

                self_.is_readable = false;
            }

            assert!(!self_.eof);

            // XXX(kinetik): work around tokio_named_pipes assuming at least 1kB available.
            self_.read_buf.reserve(INITIAL_CAPACITY);

            // Otherwise, try to read more data and try again. Make sure we've
            // got room for at least one byte to read to ensure that we don't
            // get a spurious 0 that looks like EOF
            // TODO: Probably wrong
            if let Ok(0) = futures::ready!(Pin::new(&mut self_.io).poll_read(cx, &mut self_.read_buf)) {
                self_.eof = true;
            }

            self_.is_readable = true;
        }
    }
}

impl<A, C> Sink<C::In> for Framed<A, C>
where
    A: AsyncWrite + Unpin,
    C: Codec + Unpin,
{
    type Error = io::Error;

    fn start_send(mut self: Pin<&mut Self>, item: C::In) -> Result<(), Self::Error> {
        // If the buffer is already over BACKPRESSURE_THRESHOLD,
        // then attempt to flush it. If after flush it's *still*
        // over BACKPRESSURE_THRESHOLD, then reject the send.
        /* TODO: poll_ready
        if self.write_buf.len() > BACKPRESSURE_THRESHOLD {
            self.flush();
            if self.write_buf.len() > BACKPRESSURE_THRESHOLD {
                return Ok(AsyncSink::NotReady(item));
            }
        } */

        let self_ = &mut *self;
        self_.codec.encode(item, &mut self_.write_buf)?;
        Ok(())
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        trace!("flushing framed transport");

        futures::ready!(self.do_write(cx))?;

        futures::ready!(Pin::new(&mut self.io).poll_flush(cx))?;

        trace!("framed transport flushed");
        Poll::Ready(Ok(()))
    }

    fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        todo!()
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Result<(), Self::Error>> {
        todo!();
        //futures::ready!(self.poll_flush());
        //self.io.shutdown()
    }
}

pub fn framed<A: Unpin, C: Unpin>(io: A, codec: C) -> Framed<A, C> {
    Framed {
        io,
        codec,
        read_buf: BytesMut::with_capacity(INITIAL_CAPACITY),
        write_buf: BytesMut::with_capacity(INITIAL_CAPACITY),
        frame: None,
        is_readable: false,
        eof: false,
    }
}
