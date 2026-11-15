//! TCP client stream and a server that serves stream factories over TCP.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::{ByteStream, StreamError};

const READ_CHUNK: usize = 4096;
const POLL: Duration = Duration::from_millis(50);

/// A [`ByteStream`] over a TCP connection.
#[derive(Debug)]
pub struct TcpClient {
    sock: TcpStream,
    rx: Option<Receiver<Vec<u8>>>,
    closed: bool,
    idle: Option<Duration>,
}

impl TcpClient {
    /// Connect and start the reader thread.
    ///
    /// # Errors
    /// Any connect or socket-clone error.
    pub fn connect(addr: impl ToSocketAddrs) -> std::io::Result<Self> {
        Self::from_stream(TcpStream::connect(addr)?)
    }

    /// Wrap an established connection.
    ///
    /// # Errors
    /// A socket-clone error.
    pub fn from_stream(sock: TcpStream) -> std::io::Result<Self> {
        sock.set_nodelay(true)?;
        let mut reader = sock.try_clone()?;
        let (tx, rx) = channel();
        thread::spawn(move || {
            let mut buf = [0_u8; READ_CHUNK];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        Ok(TcpClient {
            sock,
            rx: Some(rx),
            closed: false,
            idle: None,
        })
    }

    /// Report `idle` from [`ByteStream::idle_timeout`].
    #[must_use]
    pub fn with_idle_timeout(mut self, idle: Duration) -> Self {
        self.idle = Some(idle);
        self
    }
}

impl ByteStream for TcpClient {
    fn write(&mut self, bytes: &[u8]) -> Result<(), StreamError> {
        if self.closed {
            return Err(StreamError::Closed);
        }
        self.sock.write_all(bytes).map_err(StreamError::from)
    }

    fn receiver(&mut self) -> Option<Receiver<Vec<u8>>> {
        self.rx.take()
    }

    fn close(&mut self) {
        self.closed = true;
        let _ = self.sock.shutdown(Shutdown::Both);
    }

    fn idle_timeout(&self) -> Option<Duration> {
        self.idle
    }
}

/// Makes the stream that serves one accepted connection; `None` refuses the connection.
pub type Factory = dyn Fn() -> Option<Box<dyn ByteStream>> + Send + Sync;

/// A TCP server: each accepted connection gets a stream from the factory, with socket bytes written
/// to the stream and the stream's reply chunks written to the socket.
#[derive(Debug)]
pub struct Server {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    accept: Mutex<Option<JoinHandle<()>>>,
}

/// The default listen address: loopback, with the port chosen by the OS when `port` is 0.
#[must_use]
pub fn loopback(port: u16) -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, port))
}

impl Server {
    /// Listen on `addr` and serve `factory`. Use [`loopback`] for the default.
    ///
    /// # Errors
    /// Any bind error.
    pub fn start<F>(addr: impl ToSocketAddrs, factory: F) -> std::io::Result<Self>
    where
        F: Fn() -> Option<Box<dyn ByteStream>> + Send + Sync + 'static,
    {
        let listener = TcpListener::bind(addr)?;
        let addr = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let factory: Arc<Factory> = Arc::new(factory);
        let stop2 = Arc::clone(&stop);
        let accept = thread::spawn(move || {
            for conn in listener.incoming() {
                if stop2.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(sock) = conn else { continue };
                let factory = Arc::clone(&factory);
                let stop = Arc::clone(&stop2);
                thread::spawn(move || serve_one(&sock, &*factory, &stop));
            }
        });
        Ok(Server {
            addr,
            stop,
            accept: Mutex::new(Some(accept)),
        })
    }

    /// The address actually bound.
    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Stop accepting and wait for the accept thread. Connections already open end when their
    /// peer closes, their stream ends, or within a poll interval of this call.
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the blocked accept so it sees the flag.
        let _ = TcpStream::connect(if self.addr.ip().is_unspecified() {
            loopback(self.addr.port())
        } else {
            self.addr
        });
        if let Some(h) = self.accept.lock().ok().and_then(|mut g| g.take()) {
            let _ = h.join();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn serve_one(sock: &TcpStream, factory: &Factory, stop: &AtomicBool) {
    let Some(mut stream) = factory() else {
        let _ = sock.shutdown(Shutdown::Both);
        return;
    };
    let Some(replies) = stream.receiver() else {
        let _ = sock.shutdown(Shutdown::Both);
        return;
    };
    let _ = sock.set_nodelay(true);
    let Ok(mut reader) = sock.try_clone() else {
        return;
    };
    let done = Arc::new(AtomicBool::new(false));
    let done2 = Arc::clone(&done);
    let uplink = thread::spawn(move || {
        let mut buf = [0_u8; READ_CHUNK];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 || stream.write(&buf[..n]).is_err() {
                break;
            }
        }
        stream.close();
        done2.store(true, Ordering::SeqCst);
    });
    let mut writer = sock;
    loop {
        match replies.recv_timeout(POLL) {
            Ok(chunk) => {
                if writer.write_all(&chunk).is_err() {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if done.load(Ordering::SeqCst) || stop.load(Ordering::SeqCst) {
                    break;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    let _ = sock.shutdown(Shutdown::Both);
    let _ = uplink.join();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem;

    /// A stream that answers every write with the same bytes followed by `>`.
    fn echo() -> Box<dyn ByteStream> {
        let (ours, mut theirs) = mem::pair();
        let rx = theirs.receiver().unwrap();
        thread::spawn(move || {
            while let Ok(mut r) = rx.recv() {
                r.push(b'>');
                if theirs.write(&r).is_err() {
                    break;
                }
            }
        });
        Box::new(ours)
    }

    const WAIT: Duration = Duration::from_secs(5);

    #[test]
    fn a_client_talks_to_a_served_stream() {
        let srv = Server::start(loopback(0), || Some(echo())).unwrap();
        assert!(srv.local_addr().ip().is_loopback());
        let mut c = TcpClient::connect(srv.local_addr()).unwrap();
        let rx = c.receiver().unwrap();
        c.write(b"ATI\r").unwrap();
        assert_eq!(rx.recv_timeout(WAIT).unwrap(), b"ATI\r>");
        c.close();
        assert_eq!(c.write(b"x"), Err(StreamError::Closed));
    }

    #[test]
    fn each_connection_gets_its_own_stream() {
        let srv = Server::start(loopback(0), || Some(echo())).unwrap();
        let mut a = TcpClient::connect(srv.local_addr()).unwrap();
        let mut b = TcpClient::connect(srv.local_addr()).unwrap();
        let (ra, rb) = (a.receiver().unwrap(), b.receiver().unwrap());
        a.write(b"A").unwrap();
        b.write(b"B").unwrap();
        assert_eq!(ra.recv_timeout(WAIT).unwrap(), b"A>");
        assert_eq!(rb.recv_timeout(WAIT).unwrap(), b"B>");
    }

    #[test]
    fn a_refused_factory_closes_the_connection() {
        let srv = Server::start(loopback(0), || None).unwrap();
        let mut c = TcpClient::connect(srv.local_addr()).unwrap();
        let rx = c.receiver().unwrap();
        assert_eq!(rx.recv_timeout(WAIT), Err(RecvTimeoutError::Disconnected));
    }

    #[test]
    fn the_stream_ending_closes_the_socket_after_its_last_reply() {
        let srv = Server::start(loopback(0), || {
            let (ours, mut theirs) = mem::pair();
            theirs.write(b"hi").unwrap();
            drop(theirs);
            Some(Box::new(ours) as Box<dyn ByteStream>)
        })
        .unwrap();
        let mut c = TcpClient::connect(srv.local_addr()).unwrap();
        let rx = c.receiver().unwrap();
        assert_eq!(rx.recv_timeout(WAIT).unwrap(), b"hi");
        assert_eq!(rx.recv_timeout(WAIT), Err(RecvTimeoutError::Disconnected));
    }

    #[test]
    fn shutdown_stops_accepting() {
        let srv = Server::start(loopback(0), || Some(echo())).unwrap();
        let addr = srv.local_addr();
        srv.shutdown();
        assert!(TcpStream::connect(addr).is_err());
    }
}
