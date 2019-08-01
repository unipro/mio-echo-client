use std::io::{self, Read, Write};
use std::fs::File;
use std::process;
use std::time::Duration;
use std::collections::VecDeque;
use std::os::unix::io::FromRawFd;

use mio::net::TcpStream;
use mio::{Events, Poll, PollOpt, Ready, Token};
use mio::unix::EventedFd;

use failure::Error;

const DEFAULT_BUF_SIZE: usize = 1024;

pub fn run(addr: &str) -> Result<(), Error> {
    const IN: Token = Token(0);
    const OUT: Token = Token(1);
    const SOCK: Token = Token(4);

    // Create a poll instance
    let poll = Poll::new()?;
    let mut events = Events::with_capacity(1024);

    // Setup the client socket
    let mut sock = TcpStream::connect(&addr.parse()?)?;
    poll.register(&sock, SOCK, Ready::writable(), PollOpt::edge())?;
    poll.poll(&mut events, Some(Duration::from_millis(1000)))?;
    if events.iter().next().is_none() {
        return Err(io::Error::new(io::ErrorKind::TimedOut,
                                  "connection timeout")
                   .into());
    }
    poll.reregister(&sock, SOCK, Ready::readable(), PollOpt::edge())?;

    println!("connection established to {}", sock.peer_addr()?);

    let mut in_fd : File = unsafe{FromRawFd::from_raw_fd(0)};
    let mut out_fd : File = unsafe{FromRawFd::from_raw_fd(1)};
    let in_fd_e = EventedFd(&0);
    let out_fd_e = EventedFd(&1);

    poll.register(&in_fd_e, IN, Ready::readable(), PollOpt::edge()).unwrap();
    poll.register(&out_fd_e, OUT, Ready::empty(), PollOpt::edge()).unwrap();

    let mut in_bufs: VecDeque<Vec<u8>> = VecDeque::new();
    let mut out_bufs: VecDeque<Vec<u8>> = VecDeque::new();

    let mut sent_pos = 0;
    let mut wrote_pos = 0;
    let mut sock_writable = false;
    let mut out_writable = false;

    loop {
        poll.poll(&mut events, None)?;

        for event in &events {
            match event.token() {
                IN => {
                    if event.readiness().is_readable() {
                        read_line(&mut in_fd, &mut in_bufs)?;
                    }
                }
                SOCK => {
                    if event.readiness().is_readable() {
                        read(&mut sock, &mut out_bufs)?;
                    }
                }
                _ => unreachable!(),
            }
        }

        write(&mut sock, &mut in_bufs, &mut sent_pos)?;
        if in_bufs.is_empty() == sock_writable {
            sock_writable = ! sock_writable;
            let ready = if sock_writable {
                Ready::readable() | Ready::writable()
            } else {
                Ready::readable()
            };
            poll.register(&sock, OUT, ready, PollOpt::edge())?;

        }

        write(&mut out_fd, &mut out_bufs, &mut wrote_pos)?;
        if out_bufs.is_empty() == out_writable {
            out_writable = ! out_writable;
            let ready = if out_writable {
                Ready::writable()
            } else {
                Ready::empty()
            };
            poll.register(&out_fd_e, OUT, ready, PollOpt::edge())?;

        }
    }
}

pub fn read_line<T: Read>(fd: &mut T, bufs: &mut VecDeque<Vec<u8>>) -> io::Result<usize> {
    let mut rbuf = [0; DEFAULT_BUF_SIZE];
    let len = fd.read(&mut rbuf)?; // XXX long line
    let mut buf = rbuf.to_vec();
    if len < DEFAULT_BUF_SIZE {
        unsafe {
            buf.set_len(len);
        }
    }
    bufs.push_back(buf);

    Ok(len)
}

pub fn read<T: Read>(fd: &mut T, bufs: &mut VecDeque<Vec<u8>>) -> io::Result<usize> {
    let mut tot_len = 0;
    let mut rbuf = [0; DEFAULT_BUF_SIZE];

    loop {
        match fd.read(&mut rbuf) {
            Ok(0) => return Ok(0),
            Ok(len) => {
                let mut buf = rbuf.to_vec();
                if len < DEFAULT_BUF_SIZE {
                    unsafe {
                        buf.set_len(len);
                    }
                }
                bufs.push_back(buf);
                tot_len += len;
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                // Socket is not ready anymore, stop reading
                break;
            }
            Err(e) => return Err(e),
        }
    }

    Ok(tot_len)
}

pub fn write<T: Write>(fd: &mut T, bufs: &mut VecDeque<Vec<u8>>, pos: &mut usize) -> io::Result<usize> {
    let mut tot_len = 0;

    while let Some(buf) = bufs.get(0) {
        match fd.write(&buf[*pos..]) {
            Ok(len) => {
                *pos += len;
                if buf.len() == *pos {
                    bufs.pop_front();
                    *pos = 0;
                }
                tot_len += len;
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                // Socket is not ready anymore, stop writing
                break;
            }
            Err(e) => return Err(e),
        }
    }

    Ok(tot_len)
}
