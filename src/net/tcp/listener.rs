use super::TcpStream;

use std::fmt;
use std::io;
use std::net::{self, SocketAddr};
use std::pin::Pin;

use futures::{Poll, ready};
use futures::task::LocalWaker;
use futures::stream::Stream;
use mio;

use crate::reactor::{Handle, PollEvented};


/// An I/O object representing a TCP socket listening for incoming connections.
///
/// This object can be converted into a stream of incoming connections for
/// various forms of processing.
pub struct TcpListener {
    io: PollEvented<mio::net::TcpListener>,
}

impl TcpListener {
    /// Create a new TCP listener associated with this event loop.
    ///
    /// The TCP listener will bind to the provided `addr` address, if available.
    /// If the result is `Ok`, the socket has successfully bound.
    pub fn bind(addr: &SocketAddr) -> io::Result<TcpListener> {
        let l = mio::net::TcpListener::bind(addr)?;
        Ok(TcpListener::new(l))
    }

    /// Attempt to accept a connection and create a new connected `TcpStream` if
    /// successful.
    ///
    /// Note that typically for simple usage it's easier to treat incoming
    /// connections as a `Stream` of `TcpStream`s with the `incoming` method
    /// below.
    ///
    /// # Return
    ///
    /// On success, returns `Ok(Async::Ready((socket, addr)))`.
    ///
    /// If the listener is not ready to accept, the method returns
    /// `Ok(Async::NotReady)` and arranges for the current task to receive a
    /// notification when the listener becomes ready to accept.
    ///
    /// # Panics
    ///
    /// This function will panic if called from outside of a task context.
    pub fn poll_accept(&mut self, lw: &LocalWaker) -> Poll<io::Result<(TcpStream, SocketAddr)>> {
        let (io, addr) = ready!(self.poll_accept_std(lw)?);

        let io = mio::net::TcpStream::from_stream(io)?;
        let io = TcpStream::new(io);

        Poll::Ready(Ok((io, addr)))
    }

    /// Attempt to accept a connection and create a new connected `TcpStream` if
    /// successful.
    ///
    /// This function is the same as `accept` above except that it returns a
    /// `std::net::TcpStream` instead of a `tokio::net::TcpStream`. This in turn
    /// can then allow for the TCP stream to be associated with a different
    /// reactor than the one this `TcpListener` is associated with.
    ///
    /// # Return
    ///
    /// On success, returns `Ok(Async::Ready((socket, addr)))`.
    ///
    /// If the listener is not ready to accept, the method returns
    /// `Ok(Async::NotReady)` and arranges for the current task to receive a
    /// notification when the listener becomes ready to accept.
    ///
    /// # Panics
    ///
    /// This function will panic if called from outside of a task context.
    pub fn poll_accept_std(&mut self, lw: &LocalWaker)
        -> Poll<io::Result<(net::TcpStream, SocketAddr)>>
    {
        ready!(self.io.poll_read_ready(lw)?);

        match self.io.get_ref().accept_std() {
            Ok(pair) => Poll::Ready(Ok(pair)),
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                self.io.clear_read_ready(lw)?;
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e)),
        }
    }

    /// Create a new TCP listener from the standard library's TCP listener.
    ///
    /// This method can be used when the `Handle::tcp_listen` method isn't
    /// sufficient because perhaps some more configuration is needed in terms of
    /// before the calls to `bind` and `listen`.
    ///
    /// This API is typically paired with the `net2` crate and the `TcpBuilder`
    /// type to build up and customize a listener before it's shipped off to the
    /// backing event loop. This allows configuration of options like
    /// `SO_REUSEPORT`, binding to multiple addresses, etc.
    ///
    /// The `addr` argument here is one of the addresses that `listener` is
    /// bound to and the listener will only be guaranteed to accept connections
    /// of the same address type currently.
    ///
    /// Finally, the `handle` argument is the event loop that this listener will
    /// be bound to.
    /// Use `Handle::default()` to lazily bind to an event loop, just like `bind` does.
    ///
    /// The platform specific behavior of this function looks like:
    ///
    /// * On Unix, the socket is placed into nonblocking mode and connections
    ///   can be accepted as normal
    ///
    /// * On Windows, the address is stored internally and all future accepts
    ///   will only be for the same IP version as `addr` specified. That is, if
    ///   `addr` is an IPv4 address then all sockets accepted will be IPv4 as
    ///   well (same for IPv6).
    pub fn from_std(listener: net::TcpListener, handle: &Handle)
        -> io::Result<TcpListener>
    {
        let io = mio::net::TcpListener::from_std(listener)?;
        let io = PollEvented::new_with_handle(io, handle)?;
        Ok(TcpListener { io })
    }

    fn new(listener: mio::net::TcpListener) -> TcpListener {
        let io = PollEvented::new(listener);
        TcpListener { io }
    }

    /// Returns the local address that this listener is bound to.
    ///
    /// This can be useful, for example, when binding to port 0 to figure out
    /// which port was actually bound.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.io.get_ref().local_addr()
    }

    /// Consumes this listener, returning a stream of the sockets this listener
    /// accepts.
    ///
    /// This method returns an implementation of the `Stream` trait which
    /// resolves to the sockets the are accepted on this listener.
    ///
    /// # Errors
    ///
    /// Note that accepting a connection can lead to various errors and not all of them are
    /// necessarily fatal ‒ for example having too many open file descriptors or the other side
    /// closing the connection while it waits in an accept queue. These would terminate the stream
    /// if not handled in any way.
    ///
    /// If aiming for production, decision what to do about them must be made. The
    /// [`tk-listen`](https://crates.io/crates/tk-listen) crate might be of some help.
    pub fn incoming(self) -> Incoming {
        Incoming::new(self)
    }

    /// Gets the value of the `IP_TTL` option for this socket.
    ///
    /// For more information about this option, see [`set_ttl`].
    ///
    /// [`set_ttl`]: #method.set_ttl
    pub fn ttl(&self) -> io::Result<u32> {
        self.io.get_ref().ttl()
    }

    /// Sets the value for the `IP_TTL` option on this socket.
    ///
    /// This value sets the time-to-live field that is used in every packet sent
    /// from this socket.
    pub fn set_ttl(&self, ttl: u32) -> io::Result<()> {
        self.io.get_ref().set_ttl(ttl)
    }
}

impl fmt::Debug for TcpListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.io.get_ref().fmt(f)
    }
}

#[cfg(unix)]
mod sys {
    use std::os::unix::prelude::*;
    use super::TcpListener;

    impl AsRawFd for TcpListener {
        fn as_raw_fd(&self) -> RawFd {
            self.io.get_ref().as_raw_fd()
        }
    }
}

#[cfg(windows)]
mod sys {
    // TODO: let's land these upstream with mio and then we can add them here.
    //
    // use std::os::windows::prelude::*;
    // use super::{TcpListener;
    //
    // impl AsRawHandle for TcpListener {
    //     fn as_raw_handle(&self) -> RawHandle {
    //         self.listener.io().as_raw_handle()
    //     }
    // }
}

/// Stream returned by the `TcpListener::incoming` function representing the
/// stream of sockets received from a listener.
#[must_use = "streams do nothing unless polled"]
#[derive(Debug)]
pub struct Incoming {
    inner: TcpListener,
}

impl Incoming {
    pub(crate) fn new(listener: TcpListener) -> Incoming {
        Incoming { inner: listener }
    }
}

impl Stream for Incoming {
    type Item = io::Result<TcpStream>;

    fn poll_next(mut self: Pin<&mut Self>, lw: &LocalWaker) -> Poll<Option<Self::Item>> {
        let (socket, _) = ready!(self.inner.poll_accept(lw)?);
        Poll::Ready(Some(Ok(socket)))
    }
}