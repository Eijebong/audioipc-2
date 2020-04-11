// Copied from tokio-uds/src/stream.rs revision 25e835c5b7e2cfeb9c22b1fd576844f6814a9477 (tokio-uds 0.2.5)
// License MIT per upstream: https://github.com/tokio-rs/tokio/blob/master/tokio-uds/LICENSE
// - Removed ucred for build simplicity
// - Added clear_{read,write}_ready per: https://github.com/tokio-rs/tokio/pull/1294

use futures_io::{AsyncRead, AsyncWrite};
use tokio::io::{AsyncWriteExt};
use tokio::runtime::Handle;
use tokio::io::PollEvented;
use std::task::{Context, Poll};
use std::pin::Pin;

use bytes::{Buf, BufMut, buf::IoSliceMut};
use futures::Future;
use std::io::IoSlice;
use libc;
use mio::Ready;
use mio_uds;

use std::fmt;
use std::io::{self};
use std::net::Shutdown;
use std::os::unix::io::{AsRawFd, RawFd};
use std::os::unix::net::{self, SocketAddr};
use std::path::Path;

/// A structure representing a connected Unix socket.
///
/// This socket can be connected directly with `UnixStream::connect` or accepted
/// from a listener with `UnixListener::incoming`. Additionally, a pair of
/// anonymous Unix sockets can be created with `UnixStream::pair`.
pub struct UnixStream {
    io: PollEvented<mio_uds::UnixStream>,
}

/// Future returned by `UnixStream::connect` which will resolve to a
/// `UnixStream` when the stream is connected.
#[derive(Debug)]
pub struct ConnectFuture {
    inner: State,
}

#[derive(Debug)]
enum State {
    Waiting(UnixStream),
    Error(io::Error),
    Empty,
}

impl UnixStream {
    /// Connects to the socket named by `path`.
    ///
    /// This function will create a new Unix socket and connect to the path
    /// specified, associating the returned stream with the default event loop's
    /// handle.
    pub fn connect<P>(path: P) -> Result<ConnectFuture, io::Error>
    where
        P: AsRef<Path>,
    {
        let res = mio_uds::UnixStream::connect(path).map(UnixStream::new)?;

        let inner = match res {
            Ok(stream) => State::Waiting(stream),
            Err(e) => State::Error(e),
        };

        Ok(ConnectFuture { inner })
    }

    /// Consumes a `UnixStream` in the standard library and returns a
    /// nonblocking `UnixStream` from this crate.
    ///
    /// The returned stream will be associated with the given event loop
    /// specified by `handle` and is ready to perform I/O.
    pub fn from_std(stream: net::UnixStream, handle: &Handle) -> io::Result<UnixStream> {
        let stream = mio_uds::UnixStream::from_stream(stream)?;
        let io = PollEvented::new(stream)?;

        Ok(UnixStream { io })
    }

    /// Creates an unnamed pair of connected sockets.
    ///
    /// This function will create a pair of interconnected Unix sockets for
    /// communicating back and forth between one another. Each socket will
    /// be associated with the default event loop's handle.
    pub fn pair() -> io::Result<(UnixStream, UnixStream)> {
        let (a, b) = mio_uds::UnixStream::pair()?;
        let a = UnixStream::new(a)?;
        let b = UnixStream::new(b)?;

        Ok((a, b))
    }

    pub(crate) fn new(stream: mio_uds::UnixStream) -> Result<UnixStream, io::Error> {
        let io = PollEvented::new(stream)?;
        Ok(UnixStream { io })
    }

    /// Test whether this socket is ready to be read or not.
    pub fn poll_read_ready(&self, ready: Ready, cx: &mut Context) -> Poll<Result<Ready, io::Error>> {
        self.io.poll_read_ready(cx, ready)
    }

    /// Clear read ready state.
    pub fn clear_read_ready(&self, ready: mio::Ready, cx: &mut Context) -> io::Result<()> {
        self.io.clear_read_ready(cx, ready)
    }

    /// Test whether this socket is ready to be written to or not.
    pub fn poll_write_ready(&self, cx: &mut Context) -> Poll<Result<Ready, io::Error>> {
        self.io.poll_write_ready(cx)
    }

    /// Clear write ready state.
    pub fn clear_write_ready(&self, cx: &mut Context) -> io::Result<()> {
        self.io.clear_write_ready(cx)
    }

    /// Returns the socket address of the local half of this connection.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.io.get_ref().local_addr()
    }

    /// Returns the socket address of the remote half of this connection.
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.io.get_ref().peer_addr()
    }

    /// Returns the value of the `SO_ERROR` option.
    pub fn take_error(&self) -> io::Result<Option<io::Error>> {
        self.io.get_ref().take_error()
    }

    /// Shuts down the read, write, or both halves of this connection.
    ///
    /// This function will cause all pending and future I/O calls on the
    /// specified portions to immediately return with an appropriate value
    /// (see the documentation of `Shutdown`).
    pub fn shutdown(&self, how: Shutdown) -> io::Result<()> {
        self.io.get_ref().shutdown(how)
    }
}

/* TODO
impl Read for UnixStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.io.read(buf)
    }
}

impl Write for UnixStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.io.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.io.flush()
    }
} */

impl AsyncRead for UnixStream {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context, buf: &mut [u8]) -> Poll<Result<usize, io::Error>> {
        <&UnixStream>::poll_read(unsafe { self.map_unchecked_mut(|x| ::std::mem::transmute(x)) }, cx, buf)
    }
}

impl AsyncWrite for UnixStream {
    fn poll_close(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), io::Error>> {
        <&UnixStream>::poll_close(unsafe { self.map_unchecked_mut(|x| ::std::mem::transmute(x)) }, cx)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<(), io::Error>> {
        <&UnixStream>::poll_flush(unsafe { self.map_unchecked_mut(|x| ::std::mem::transmute(x)) }, cx)
    }

    fn poll_write(self: Pin<&mut Self>, cx: &mut Context, buf: &[u8]) -> Poll<Result<usize, io::Error>> {
        <&UnixStream>::poll_write(unsafe { self.map_unchecked_mut(|x| ::std::mem::transmute(x)) }, cx, buf)
    }
}

/* TODO
impl<'a> Read for &'a UnixStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        (&self.io).read(buf)
    }
}

impl<'a> Write for &'a UnixStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (&self.io).write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        (&self.io).flush()
    }
}*/

impl<'a> AsyncRead for &'a UnixStream {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context, mut buf: &mut [u8]) -> Poll<Result<usize, io::Error>> {
        if let Poll::Pending = <UnixStream>::poll_read_ready(*self, Ready::readable(), cx)? {
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        unsafe {
            let r = read_ready(&mut buf, self.as_raw_fd());
            if r == -1 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::WouldBlock {
                    self.io.clear_read_ready(cx, Ready::readable())?;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                } else {
                    Poll::Ready(Err(e))
                }
            } else {
                //let r = r as usize;
                //buf.advance_mut(r);
                Poll::Ready(Ok(r as usize))
            }
        }
    }
}

impl<'a> AsyncWrite for &'a UnixStream {
    fn poll_close(self: Pin<&mut Self>, _: &mut Context) -> Poll<Result<(), io::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context) -> Poll<Result<(), io::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_write(self: Pin<&mut Self>, cx: &mut Context, buf: &[u8]) -> Poll<Result<usize, io::Error>> {
        if let Poll::Pending = <UnixStream>::poll_write_ready(*self, cx)? {
            cx.waker().wake_by_ref();
            return Poll::Pending
        }
        unsafe {
            let r = write_ready(&buf, self.as_raw_fd());
            if r == -1 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::WouldBlock {
                    self.io.clear_write_ready(cx)?;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                } else {
                    Poll::Ready(Err(e))
                }
            } else {
                //let r = r as usize;
                //buf.advance(r);
                Poll::Ready(Ok(r as usize))
            }
        }
    }
}

impl fmt::Debug for UnixStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.io.get_ref().fmt(f)
    }
}

impl AsRawFd for UnixStream {
    fn as_raw_fd(&self) -> RawFd {
        self.io.get_ref().as_raw_fd()
    }
}

impl Future for ConnectFuture {
    type Output = Result<UnixStream, io::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Result<UnixStream, io::Error>> {
        use std::mem;

        match self.inner {
            State::Waiting(ref mut stream) => {
                if let Poll::Pending = stream.io.poll_write_ready(cx)? {
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }

                if let Some(e) = stream.io.get_ref().take_error()? {
                    return Poll::Ready(Err(e));
                }
            }
            State::Error(_) => {
                let e = match mem::replace(&mut self.inner, State::Empty) {
                    State::Error(e) => e,
                    _ => unreachable!(),
                };

                return Poll::Ready(Err(e));
            }
            State::Empty => panic!("can't poll stream twice"),
        }

        match mem::replace(&mut self.inner, State::Empty) {
            State::Waiting(stream) => Poll::Ready(Ok(stream)),
            _ => unreachable!(),
        }
    }
}

unsafe fn read_ready<B: BufMut>(buf: &mut B, raw_fd: RawFd) -> isize {
    // The `IoVec` type can't have a 0-length size, so we create a bunch
    // of dummy versions on the stack with 1 length which we'll quickly
    // overwrite.
    let b1: &mut [u8] = &mut [0];
    let b2: &mut [u8] = &mut [0];
    let b3: &mut [u8] = &mut [0];
    let b4: &mut [u8] = &mut [0];
    let b5: &mut [u8] = &mut [0];
    let b6: &mut [u8] = &mut [0];
    let b7: &mut [u8] = &mut [0];
    let b8: &mut [u8] = &mut [0];
    let b9: &mut [u8] = &mut [0];
    let b10: &mut [u8] = &mut [0];
    let b11: &mut [u8] = &mut [0];
    let b12: &mut [u8] = &mut [0];
    let b13: &mut [u8] = &mut [0];
    let b14: &mut [u8] = &mut [0];
    let b15: &mut [u8] = &mut [0];
    let b16: &mut [u8] = &mut [0];
    let mut bufs: [IoSliceMut; 16] = [
        IoSliceMut::from(b1),
        IoSliceMut::from(b2),
        IoSliceMut::from(b3),
        IoSliceMut::from(b4),
        IoSliceMut::from(b5),
        IoSliceMut::from(b6),
        IoSliceMut::from(b7),
        IoSliceMut::from(b8),
        IoSliceMut::from(b9),
        IoSliceMut::from(b10),
        IoSliceMut::from(b11),
        IoSliceMut::from(b12),
        IoSliceMut::from(b13),
        IoSliceMut::from(b14),
        IoSliceMut::from(b15),
        IoSliceMut::from(b16),
    ];

    let n = buf.bytes_vectored_mut(&mut bufs);
    read_ready_vecs(&mut bufs[..n], raw_fd)
}

unsafe fn read_ready_vecs(bufs: &mut [IoSliceMut], raw_fd: RawFd) -> isize {
    // TODO: Check
    let iovecs: &[libc::iovec] = std::mem::transmute(bufs);

    libc::readv(raw_fd, iovecs.as_ptr(), iovecs.len() as i32)
}

unsafe fn write_ready<B: Buf>(buf: &B, raw_fd: RawFd) -> isize {
    // The `IoVec` type can't have a zero-length size, so create a dummy
    // version from a 1-length slice which we'll overwrite with the
    // `bytes_vec` method.
    static DUMMY: &[u8] = &[];
    let iovec = IoSlice::new(DUMMY);
    let mut bufs = [
        iovec, iovec, iovec, iovec, iovec, iovec, iovec, iovec, iovec, iovec, iovec, iovec, iovec,
        iovec, iovec, iovec,
    ];

    let n = buf.bytes_vectored(&mut bufs);
    write_ready_vecs(&bufs[..n], raw_fd)
}

unsafe fn write_ready_vecs(bufs: &[IoSlice], raw_fd: RawFd) -> isize {
    let iovecs: &[libc::iovec] = std::mem::transmute(bufs);

    libc::writev(raw_fd, iovecs.as_ptr(), iovecs.len() as i32)
}
