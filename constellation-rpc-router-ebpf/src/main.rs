#![no_std]
#![no_main]

use aya_ebpf::{
    bindings::xdp_action,
    macros::{map, xdp},
    maps::XskMap,
    programs::XdpContext,
};
use aya_log_ebpf::info;

use core::mem;

use network_types::{
    eth::{EthHdr, EtherType},
    ip::{IpProto, Ipv4Hdr},
    udp::UdpHdr,
};


pub const RPC_PORT: u16 = 8000;

#[map(name = "XSK_MAP")]
static XSK_MAP: XskMap = XskMap::with_max_entries(1, 0);

#[inline(always)]
fn ptr_at<T>(ctx: &XdpContext, offset: usize) -> Result<*const T, ()> {
    let start = ctx.data();
    let end = ctx.data_end();
    let len = mem::size_of::<T>();

    if start + offset + len > end {
        return Err(());
    }

    Ok((start + offset) as *const T)
}

#[xdp]
pub fn constellation_rpc_router(ctx: XdpContext) -> u32 {
    match try_constellation_rpc_router(ctx) {
        Ok(ret) => ret,
        Err(_) => xdp_action::XDP_ABORTED,
    }
}

fn try_constellation_rpc_router(ctx: XdpContext) -> Result<u32, ()> {
    let eth_hdr: *const EthHdr = ptr_at(&ctx, 0)?;
    if unsafe { (*eth_hdr).ether_type() } != Ok(EtherType::Ipv4) {
        return Ok(xdp_action::XDP_PASS);
    }

    let ipv4_hdr: *const Ipv4Hdr = ptr_at(&ctx, EthHdr::LEN)?;
    let proto = match unsafe { (*ipv4_hdr).proto() } {
        Ok(p) => p,
        Err(_) => return Ok(xdp_action::XDP_PASS),
    };

    if proto != IpProto::Udp {
        return Ok(xdp_action::XDP_PASS);
    }

    let ihl = unsafe { (*ipv4_hdr).ihl() as usize };
    if ihl < Ipv4Hdr::LEN {
        return Ok(xdp_action::XDP_PASS);
    }

    let udp_hdr: *const UdpHdr = ptr_at(&ctx, EthHdr::LEN + ihl)?;

    let src_addr = unsafe { (*ipv4_hdr).src_addr() };
    let dst_port = unsafe { (*udp_hdr).dst_port() };
    if dst_port != RPC_PORT {
        return Ok(xdp_action::XDP_PASS);
    }

    info!(
        &ctx,
        "RX UDP packet: SRC={:i}, DST_PORT={}", src_addr, dst_port
    );

    let action = XSK_MAP.redirect(0, 0).unwrap_or(xdp_action::XDP_PASS);
    if action == xdp_action::XDP_REDIRECT {
        info!(&ctx, "Redirecting to AF_XDP socket...");
    }

    Ok(action)
}

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[unsafe(link_section = "license")]
#[unsafe(no_mangle)]
static LICENSE: [u8; 13] = *b"Dual MIT/GPL\0";
