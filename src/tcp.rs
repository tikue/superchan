//! Module `tcp` provides support for channels that communicate
//! over TCP.

use serialize::{Decodable, Encodable};
use serialize::json::{Decoder, DecoderError, decode, Encoder};
use std::comm;
use std::io::{IoError, IoResult, TcpStream};
use std::io::IoErrorKind::{EndOfFile, TimedOut};
use std::io::net::ip::ToSocketAddr;
use std::io::net::tcp::TcpAcceptor;
use std::sync::Future;
use super::{SendRequest, ReceiverError};

/// `TcpSender` is the sender half of a TCP connection.
pub struct TcpSender<T>(comm::Sender<SendRequest<T>>);

impl<T> TcpSender<T> where T: Encodable<Encoder<'static>, IoError> + Send {
    /// Create a new TcpSender (aka TCP client) for the specified address. It will
    /// fail if no TcpReceiver (aka TCP server) is waiting to receive the connection.
    #[allow(unused_must_use)]
    pub fn new<A: ToSocketAddr>(addr: A) -> IoResult<TcpSender<T>> {
        let (tx, rx) = comm::channel::<SendRequest<T>>();
        let mut stream = try!(TcpStream::connect(addr));
        spawn(proc() {
            for (t, fi) in rx.iter() {
                let e = Encoder::buffer_encode(&t);
                let result = send(&mut stream, e);
                fi.send_opt(result);
            }
        });
        Ok(TcpSender(tx))
    }
}

fn send(stream: &mut TcpStream, data: Vec<u8>) -> IoResult<()> {
    try!(stream.write_le_uint(data.len()));
    try!(stream.write(data.as_slice()));
    try!(stream.flush());
    Ok(())
}

impl<T> super::Sender<T> for TcpSender<T> where T: Encodable<Encoder<'static>, IoError> + Send {
    /// Non-blocking send along the channel.
    fn send(&mut self, t: T) -> Future<IoResult<()>> {
        let (fi, fo) = comm::channel();
        self.0.send((t, fi));
        Future::from_receiver(fo)
    }
}

/// TcpReceiver is the receiver half of a TCP connection.
pub struct TcpReceiver<T> {
    streams: Receiver<TcpStream>,
    acceptor: TcpAcceptor,
}

impl<T> TcpReceiver<T> {
    /// Create a new TcpReceiver (aka TCP server) bound to the specified address.
    #[allow(unused_must_use)]
    pub fn new<A: ToSocketAddr>(addr: A) -> IoResult<TcpReceiver<T>> {
        use std::io::{Acceptor, Listener};
        use std::io::net::tcp::TcpListener;

        let (tx, rx) = channel();

        let listener = try!(TcpListener::bind(addr));
        let acceptor = try!(listener.listen());
        let mut acceptor2 = acceptor.clone();
        spawn(proc() {
            for stream in acceptor2.incoming() {
                match stream {
                    Ok(stream) => tx.send(stream),
                    Err(IoError{kind: EndOfFile, ..}) => return,
                    Err(e) => panic!("{}", e),
                }
            }
        });

        Ok(TcpReceiver{ streams: rx, acceptor: acceptor, })
    }

}

fn get_size(stream: &mut TcpStream) -> Result<uint, ReceiverError> {
    loop {
        stream.set_read_timeout(Some(10)); // TODO: set this value as a config?
        match stream.read_le_uint() {
            Ok(size) => return Ok(size),
            Err(IoError{kind: TimedOut, ..}) => continue,
            Err(e) => return Err(ReceiverError::IoError(e)),
        }
    }
}

impl<T> super::Receiver<T> for TcpReceiver<T> where T: Decodable<Decoder, DecoderError> {
    /// Attempt to receive a value on the channel. This method blocks until a value
    /// is available.
    fn try_recv(&mut self) -> Result<T, ReceiverError> {
        let mut stream = self.streams.recv();
        let size = try!(get_size(&mut stream));
        let data = try!(stream.read_exact(size));
        let string = try!(String::from_utf8(data));
        let decoded = try!(decode::<T>(string.as_slice()));
        Ok(decoded)
    }
}

#[unsafe_destructor]
impl<T> Drop for TcpReceiver<T> {
    #[allow(unused_must_use)]
    fn drop(&mut self) {
        self.acceptor.close_accept();
    }
}

#[cfg(test)]
mod test {
    use super::super::{Sender, Receiver};
    use super::{TcpSender, TcpReceiver};

    #[deriving(Encodable, Decodable)]
    enum MyEnum {
        NoValue,
        IntValue(int),
    }

    #[test]
    fn send_and_recv_string() {
        const ADDR: &'static str = "127.0.0.1:8080";
        let mut receiver: TcpReceiver<String> = TcpReceiver::new(ADDR).unwrap();
        let mut sender: TcpSender<String> = TcpSender::new(ADDR).unwrap();

        sender.send("hello superchan!".into_string());
        assert_eq!("hello superchan!".into_string(), receiver.recv());
    }

    #[test]
    fn send_and_recv_int() {
        const ADDR: &'static str = "127.0.0.1:8081";
        let mut receiver: TcpReceiver<int> = TcpReceiver::new(ADDR).unwrap();
        let mut sender: TcpSender<int> = TcpSender::new(ADDR).unwrap();

        sender.send(-13);
        assert_eq!(-13, receiver.recv());
    }

    #[test]
    fn send_and_recv_custom() {
        const ADDR: &'static str = "127.0.0.1:8082";
        let mut receiver: TcpReceiver<MyEnum> = TcpReceiver::new(ADDR).unwrap();
        let mut sender: TcpSender<MyEnum> = TcpSender::new(ADDR).unwrap();

        sender.send(MyEnum::IntValue(3));
        match receiver.recv() {
            MyEnum::IntValue(val) => assert_eq!(val, 3),
            _ => panic!("received unexpected MyEnum value"),
        }
    }

    #[test]
    fn multi_send() {
        const ADDR: &'static str = "127.0.0.1:8083";
        let mut receiver: TcpReceiver<int> = TcpReceiver::new(ADDR).unwrap();
        let mut sender1: TcpSender<int> = TcpSender::new(ADDR).unwrap();
        let mut sender2: TcpSender<int> = TcpSender::new(ADDR).unwrap();

        sender1.send(1);
        sender2.send(2);
        let (val1, val2) = (receiver.recv(), receiver.recv());
        assert!((val1 == 1 && val2 == 2) || (val1 == 2 && val2 == 1));
    }
}
