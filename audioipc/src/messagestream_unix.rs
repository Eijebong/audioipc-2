// Copyright © 2017 Mozilla Foundation
//
// This program is made available under an ISC-style license.  See the
// accompanying file LICENSE for details

use super::tokio_uds_stream as tokio_uds;
use std::task::{Context, Poll};
use std::pin::Pin;
use mio::Ready;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::os::unix::net;
use futures_io::{AsyncRead, AsyncWrite};
use tokio::runtime::Handle;

#[derive(Debug)]
pub struct MessageStream(net::UnixStream);
pub struct AsyncMessageStream(tokio_uds::UnixStream);

impl MessageStream {
    fn new(stream: net::UnixStream) -> MessageStream {
        MessageStream(stream)
    }

    pub fn anonymous_ipc_pair(
    ) -> std::result::Result<(MessageStream, MessageStream), std::io::Error> {
        let pair = net::UnixStream::pair()?;
        Ok((MessageStream::new(pair.0), MessageStream::new(pair.1)))
    }

    pub unsafe fn from_raw_fd(raw: super::PlatformHandleType) -> MessageStream {
        MessageStream::new(net::UnixStream::from_raw_fd(raw))
    }

    pub fn into_tokio_ipc(
        self,
        handle: &Handle,
    ) -> std::result::Result<AsyncMessageStream, std::io::Error> {
        Ok(AsyncMessageStream::new(tokio_uds::UnixStream::from_std(
            self.0, handle,
        )?))
    }
}

impl IntoRawFd for MessageStream {
    fn into_raw_fd(self) -> RawFd {
        self.0.into_raw_fd()
    }
}

impl AsyncMessageStream {
    fn new(stream: tokio_uds::UnixStream) -> AsyncMessageStream {
        AsyncMessageStream(stream)
    }

    pub fn poll_read_ready(&self, ready: Ready, cx: &mut Context) -> Poll<Result<Ready, std::io::Error>> {
        self.0.poll_read_ready(ready, cx)
    }

    pub fn clear_read_ready(&self, ready: Ready, cx: &mut Context) -> Result<(), std::io::Error> {
        self.0.clear_read_ready(ready, cx)
    }

    pub fn poll_write_ready(&self, cx: &mut Context) -> Poll<Result<Ready, std::io::Error>> {
        self.0.poll_write_ready(cx)
    }

    pub fn clear_write_ready(&self, cx: &mut Context) -> Result<(), std::io::Error> {
        self.0.clear_write_ready(cx)
    }
}

/* TODO
impl std::io::Read for AsyncMessageStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl std::io::Write for AsyncMessageStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
} */

impl AsyncRead for AsyncMessageStream {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context, buf: &mut [u8]) -> Poll<Result<usize, std::io::Error>> {
        <&tokio_uds::UnixStream>::poll_read(Pin::new(&mut &self.0), cx, buf)
    }
}

impl AsyncWrite for AsyncMessageStream {
    fn poll_close(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), std::io::Error>> {
        <&tokio_uds::UnixStream>::poll_close(Pin::new(&mut &self.0), cx)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), std::io::Error>> {
        <&tokio_uds::UnixStream>::poll_flush(Pin::new(&mut &self.0), cx)
    }

    fn poll_write(self: Pin<&mut Self>, cx: &mut Context, buf: &[u8]) -> Poll<Result<usize, std::io::Error>> {
        <&tokio_uds::UnixStream>::poll_write(Pin::new(&mut &self.0), cx, buf)
    }
}

impl AsRawFd for AsyncMessageStream {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}
