use std::io;
use std::mem::{size_of, MaybeUninit};
use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};
use std::os::fd::AsRawFd;

// Linux iptables REDIRECT: SO_ORIGINAL_DST (80)
const SO_ORIGINAL_DST: i32 = 80;

// SOL_IP is 0 on Linux.
const SOL_IP: i32 = 0;

#[repr(C)]
struct InAddr {
    s_addr: u32,
}

#[repr(C)]
struct SockAddrIn {
    sin_family: u16,
    sin_port: u16,
    sin_addr: InAddr,
    sin_zero: [u8; 8],
}

extern "C" {
    fn getsockopt(
        sockfd: i32,
        level: i32,
        optname: i32,
        optval: *mut core::ffi::c_void,
        optlen: *mut u32,
    ) -> i32;
}

pub fn original_dst(stream: &impl AsRawFd) -> io::Result<SocketAddrV4> {
    let fd = stream.as_raw_fd();
    let mut addr = MaybeUninit::<SockAddrIn>::zeroed();
    let mut len: u32 = size_of::<SockAddrIn>() as u32;

    let rc = unsafe {
        getsockopt(
            fd,
            SOL_IP,
            SO_ORIGINAL_DST,
            addr.as_mut_ptr() as *mut core::ffi::c_void,
            &mut len as *mut u32,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }

    let addr = unsafe { addr.assume_init() };
    let ip_u32 = u32::from_be(addr.sin_addr.s_addr);
    let ip = Ipv4Addr::from(ip_u32);
    let port = u16::from_be(addr.sin_port);

    Ok(SocketAddrV4::new(ip, port))
}

#[allow(dead_code)]
pub fn original_dst_ip(stream: &impl AsRawFd) -> io::Result<(IpAddr, u16)> {
    let dst = original_dst(stream)?;
    Ok((IpAddr::V4(*dst.ip()), dst.port()))
}

// Tokio 版本
pub fn original_dst_tokio(stream: &tokio::net::TcpStream) -> io::Result<SocketAddrV4> {
    original_dst(stream)
}

