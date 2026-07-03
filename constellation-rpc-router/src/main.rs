use anyhow::Context as _;
use aya::{
    maps::XskMap,
    programs::{Xdp, XdpMode},
};
use clap::Parser;
use xsk_rs::{
    FrameDesc, Socket,
    config::{LibxdpFlags, SocketConfig, UmemConfig},
    umem::Umem,
};
#[rustfmt::skip]
use log::{debug, warn, info};
use std::os::unix::io::AsRawFd;

use tokio::signal;

use rand;
use std::time::Duration;

#[derive(Debug, Parser)]
struct Opt {
    #[clap(short, long, default_value = "veth0")]
    iface: String,

    #[clap(short, long, default_value = "0")]
    queue_id: u32,
}

#[derive(Clone)]
struct Proposer {
    id: u16,
    addr: std::net::Ipv4Addr,
    port: u16,
}

const MAX_PROPOSERS: usize = 16;
const TIME_CHANGE_PROPOSER: Duration = Duration::from_secs(10);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let opt = Opt::parse();

    env_logger::init();

    let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let running_handle = std::sync::Arc::clone(&running);

    let rlim = libc::rlimit {
        rlim_cur: libc::RLIM_INFINITY,
        rlim_max: libc::RLIM_INFINITY,
    };
    let ret = unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &rlim) };
    if ret != 0 {
        debug!("remove limit on locked memory failed, ret is: {ret}");
    }

    let mut ebpf = aya::Ebpf::load(aya::include_bytes_aligned!(concat!(
        env!("OUT_DIR"),
        "/constellation-rpc-router"
    )))?;
    match aya_log::EbpfLogger::init(&mut ebpf) {
        Err(e) => {
            warn!("failed to initialize eBPF logger: {e}");
        }
        Ok(logger) => {
            let mut logger =
                tokio::io::unix::AsyncFd::with_interest(logger, tokio::io::Interest::READABLE)?;
            tokio::task::spawn(async move {
                loop {
                    let mut guard = logger.readable_mut().await.unwrap();
                    guard.get_inner_mut().flush();
                    guard.clear_ready();
                }
            });
        }
    }

    let Opt { iface, queue_id } = opt;

    let program: &mut Xdp = ebpf
        .program_mut("constellation_rpc_router")
        .unwrap()
        .try_into()?;
    program.load()?;
    program.attach(&iface, XdpMode::Default)?;

    let (umem, mut descs) = Umem::new(UmemConfig::default(), 32.try_into().unwrap(), false)
        .context("failed to create UMEM")?;

    let socket_config = SocketConfig::builder()
        .libxdp_flags(LibxdpFlags::XSK_LIBXDP_FLAGS_INHIBIT_PROG_LOAD)
        .build();

    let (mut tx_q, mut rx_q, fq_cq) = unsafe {
        Socket::new(
            socket_config,
            &umem,
            &iface.parse().context("failed to parse interface name")?,
            queue_id,
        )
    }
    .context("failed to bind AF_XDP socket")?;

    let (mut fq, mut cq) = fq_cq.context("missing fill/completion queue")?;

    unsafe {
        fq.produce(&descs);
    }

    let socket_fd = rx_q.fd().as_raw_fd();

    let mut xsk_map = XskMap::try_from(ebpf.map_mut("XSK_MAP").context("XSK_MAP not found")?)?;
    xsk_map
        .set(0, socket_fd, 0)
        .context("failed to set socket FD in XSK_MAP")?;

    let active_proposers = std::sync::Arc::new(std::sync::RwLock::new(
        (0..MAX_PROPOSERS)
            .map(|i| Proposer {
                id: i as u16,
                addr: "127.0.0.1".parse().unwrap(),
                port: 8000 + i as u16,
            })
            .collect::<Vec<Proposer>>(),
    ));

    let proposers_rotation_handle = std::sync::Arc::clone(&active_proposers);
    let proposers_handle = std::sync::Arc::clone(&active_proposers);

    let (rx_descs, tx_descs) = descs.split_at_mut(16);
    let rx_descs: Vec<FrameDesc> = rx_descs.to_vec();
    let mut tx_frame_pool: Vec<FrameDesc> = tx_descs.to_vec();
    let mut completed_descs: Vec<FrameDesc> = vec![FrameDesc::default(); 16];

    unsafe {
        fq.produce(&rx_descs);
    }

    let handle = tokio::task::spawn_blocking(move || {
        info!("Data Plane loop started");
        let mut tx_scratch = [0u8; 128];

        while running_handle.load(std::sync::atomic::Ordering::Relaxed) {
            let rcvd = unsafe { rx_q.poll_and_consume(&mut descs, 100) };

            match rcvd {
                Ok(count) if count > 0 => {
                    for i in 0..count {
                        let mut desc = descs[i];
                        let frame = unsafe { umem.data_mut(&mut desc) };

                        if frame.len() >= 44 {
                            let bitmap = u16::from_be_bytes([frame[42], frame[43]]);
                            let frame_len = frame.len().min(tx_scratch.len());
                            tx_scratch[..frame_len].copy_from_slice(&frame[..frame_len]);

                            if let Ok(guard) = proposers_handle.read() {
                                for p_idx in 0..MAX_PROPOSERS {
                                    if (bitmap & (1 << p_idx)) != 0 {
                                        let target = &guard[p_idx];

                                        use std::io::Write;

                                        if let Some(tx_desc) = tx_frame_pool.pop() {
                                            let mut tx_desc = tx_desc;
                                            {
                                                let mut tx_frame =
                                                    unsafe { umem.data_mut(&mut tx_desc) };
                                                tx_frame
                                                    .cursor()
                                                    .write_all(&tx_scratch[..frame_len])
                                                    .expect("failed writing packet to tx frame");
                                            }

                                            {
                                                let mut tx_frame =
                                                    unsafe { umem.data_mut(&mut tx_desc) };
                                                rewrite_port_and_addr(
                                                    tx_frame.contents_mut(),
                                                    target.addr,
                                                    target.port,
                                                );
                                            }

                                            info!(
                                                "📡 TX [Bitmap: 0x{:04X}]: Routing to Proposer #{} -> {}:{}",
                                                bitmap, target.id, target.addr, target.port
                                            );

                                            unsafe {
                                                let _ = tx_q.produce_and_wakeup(&[tx_desc]);
                                            }
                                        } else {
                                            debug!(
                                                "no free TX frame, dropping fanout to proposer #{}",
                                                p_idx
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }

                    unsafe {
                        fq.produce(&descs[..count]);
                    }
                }
                Ok(_) => {}
                Err(e) => debug!("AF_XDP poll error: {:?}", e),
            }

            let completed = unsafe { cq.consume(&mut completed_descs) };
            for d in &completed_descs[..completed] {
                tx_frame_pool.push(*d);
            }
        }
    });

    tokio::spawn(async move {
        if let Err(e) = handle.await {
            eprintln!("Data plane loop panicked: {e:?}");
        }
    });

    tokio::task::spawn(async move {
        let mut interval = tokio::time::interval(TIME_CHANGE_PROPOSER);
        loop {
            interval.tick().await;

            if let Ok(mut guard) = proposers_rotation_handle.write() {
                for proposer in guard.iter_mut() {
                    proposer.port = rand::random_range(9000..10000);
                }

                info!(
                    "[Control Plane] FULL ROTATION: All {} proposers updated",
                    MAX_PROPOSERS
                );

                debug!(
                    "Sample ports: #0: {}, #1: {}, #15: {}",
                    guard[0].port, guard[1].port, guard[15].port
                );
            }
        }
    });

    let ctrl_c = signal::ctrl_c();
    println!("Constellation RPC Router started on {}", iface);
    println!("Waiting for Ctrl-C...");
    ctrl_c.await?;
    println!("Exiting...");
    running.store(false, std::sync::atomic::Ordering::Relaxed);
    tokio::time::sleep(Duration::from_millis(100)).await;

    Ok(())
}

fn ipv4_checksum(header: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i < header.len() {
        let word = if i == 10 {
            0u16
        } else {
            u16::from_be_bytes([header[i], header[i + 1]])
        };
        sum += word as u32;
        i += 2;
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

fn rewrite_port_and_addr(data: &mut [u8], addr: std::net::Ipv4Addr, port: u16) {
    data[30..34].copy_from_slice(&addr.octets());

    let ip_header = &data[14..34]; 
    let checksum = ipv4_checksum(ip_header);
    data[24..26].copy_from_slice(&checksum.to_be_bytes());

    data[36..38].copy_from_slice(&port.to_be_bytes());
    data[40] = 0;
    data[41] = 0;
}
