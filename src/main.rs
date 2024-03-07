#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_net::tcp::client::{TcpClient, TcpClientState};
use embassy_net::udp::{PacketMetadata, UdpSocket};
use embassy_net::{Ipv4Address, Ipv4Cidr, Stack, StackResources};
use embassy_stm32::eth::generic_smi::GenericSMI;
use embassy_stm32::eth::{Ethernet, PacketQueue};
use embassy_stm32::peripherals::ETH;
use embassy_stm32::rng::Rng;
use embassy_stm32::{bind_interrupts, eth, peripherals, rng, Config};
use embassy_time::Timer;
use embedded_io_async::Write;
use embedded_nal_async::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpConnect};
use heapless::Vec;
use rand_core::RngCore;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    ETH => eth::InterruptHandler;
    RNG => rng::InterruptHandler<peripherals::RNG>;
});

type Device = Ethernet<'static, ETH, GenericSMI>;

#[embassy_executor::task]
async fn net_task(stack: &'static Stack<Device>) -> ! {
    stack.run().await
}

#[embassy_executor::main]
async fn main(spawner: Spawner) -> ! {
    let mut config = Config::default();
    {
        use embassy_stm32::rcc::*;
        config.rcc.hsi = Some(HSIPrescaler::DIV1);
        config.rcc.csi = true;
        config.rcc.hsi48 = Some(Default::default()); // needed for RNG
        config.rcc.pll1 = Some(Pll {
            source: PllSource::HSI,
            prediv: PllPreDiv::DIV4,
            mul: PllMul::MUL50,
            divp: Some(PllDiv::DIV2),
            divq: None,
            divr: None,
        });
        config.rcc.sys = Sysclk::PLL1_P; // 400 Mhz
        config.rcc.ahb_pre = AHBPrescaler::DIV2; // 200 Mhz
        config.rcc.apb1_pre = APBPrescaler::DIV2; // 100 Mhz
        config.rcc.apb2_pre = APBPrescaler::DIV2; // 100 Mhz
        config.rcc.apb3_pre = APBPrescaler::DIV2; // 100 Mhz
        config.rcc.apb4_pre = APBPrescaler::DIV2; // 100 Mhz
        config.rcc.voltage_scale = VoltageScale::Scale1;
    }
    let p = embassy_stm32::init(config);
    info!("Hello World!");

    // Generate random seed.
    let mut rng = Rng::new(p.RNG, Irqs);
    let mut seed = [0; 8];
    rng.fill_bytes(&mut seed);
    let seed = u64::from_le_bytes(seed);

    let mac_addr = [0x00, 0x00, 0xDE, 0xAD, 0xBE, 0xEF];

    static PACKETS: StaticCell<PacketQueue<16, 16>> = StaticCell::new();

    info!("Create New Device");
    let device = Ethernet::new(
        PACKETS.init(PacketQueue::<16, 16>::new()),
        p.ETH,
        Irqs,
        p.PA1,
        p.PA2,
        p.PC1,
        p.PA7,
        p.PC4,
        p.PC5,
        p.PG13,
        p.PB13,
        p.PG11,
        GenericSMI::new(0),
        mac_addr,
    );

    info!("Device created");

    // let config = embassy_net::Config::dhcpv4(Default::default());

    // Set static IP for the STM 32 card.
    let config = embassy_net::Config::ipv4_static(embassy_net::StaticConfigV4 {
        address: Ipv4Cidr::new(Ipv4Address::new(172, 16, 1, 184), 16),
        dns_servers: Vec::new(),
        gateway: Some(Ipv4Address::new(172, 16, 0, 1)),
    });

    // Init network stack
    static STACK: StaticCell<Stack<Device>> = StaticCell::new();
    static RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();
    let stack = &*STACK.init(Stack::new(
        device,
        config,
        RESOURCES.init(StackResources::<3>::new()),
        seed,
    ));

    // Launch network task
    unwrap!(spawner.spawn(net_task(stack)));

    // Ensure DHCP configuration is up before trying connect
    stack.wait_config_up().await;

    info!("Network task initialized");

    // Create a TCP client
    // let state: TcpClientState<1, 1024, 1024> = TcpClientState::new();
    // let client = TcpClient::new(&stack, &state);

    // Create a UDP client
    // Then we can use it!
    let mut rx_meta = [PacketMetadata::EMPTY; 16];
    let mut rx_buffer = [0; 4096];
    let mut tx_meta = [PacketMetadata::EMPTY; 16];
    let mut tx_buffer = [0; 4096];
    let mut buf = [0; 4096];

    let mut socket = UdpSocket::new(
        stack,
        &mut rx_meta,
        &mut rx_buffer,
        &mut tx_meta,
        &mut tx_buffer,
    );
    socket.bind(9400).unwrap();

    loop {
        let (n, ep) = socket.recv_from(&mut buf).await.unwrap();
        if let Ok(s) = core::str::from_utf8(&buf[..n]) {
            info!("ECHO (to {}): {}", ep, s);
        } else {
            info!("ECHO (to {}): bytearray len {}", ep, n);
        }
        // socket.send_to(&buf[..n], ep).await.unwrap();

        // Assume buf is a mutable byte slice and n is the number of bytes read
        let extra_text = "Extra Text"; // Your extra text
        let extra_bytes = extra_text.as_bytes();

        // Ensure the buffer is large enough to hold the extra text
        if buf.len() >= n + extra_bytes.len() {
            // Append the extra text to the end of the received data
            buf[n..n + extra_bytes.len()].copy_from_slice(extra_bytes);

            // Send the modified buffer
            socket
                .send_to(&buf[..n + extra_bytes.len()], ep)
                .await
                .unwrap();
        } else {
            // Handle error: buffer too small
            info!("Buffer is too small to add extra text.");
        }

        /*
        // You need to start a server on the host machine, for example: `nc -l 8000`
        // The IP address is the IP address of the host machine.
        let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(172, 16, 201, 12), 8000));

        info!("connecting...");
        let r = client.connect(addr).await;
        if let Err(e) = r {
            info!("connect error: {:?}", e);
            Timer::after_secs(1).await;
            continue;
        }
        let mut connection = r.unwrap();
        info!("connected!");
        loop {
            let r = connection.write_all(b"Hello\n").await;
            if let Err(e) = r {
                info!("write error: {:?}", e);
                break;
            }
            Timer::after_secs(1).await;
        }
        */
    }
}

/*
#![no_std]
#![no_main]

use clap::Parser;
use embassy_executor::{Executor, Spawner};
use embassy_net::udp::{PacketMetadata, UdpSocket};
use embassy_net::{Config, Ipv4Address, Ipv4Cidr, Stack, StackResources};
use embassy_net_tuntap::TunTapDevice;
use heapless::Vec;
use log::*;
use rand_core::{OsRng, RngCore};
use static_cell::StaticCell;


#[derive(Parser)]
#[clap(version = "1.0")]
struct Opts {
    /// TAP device name
    #[clap(long, default_value = "tap0")]
    tap: String,
    /// use a static IP instead of DHCP
    #[clap(long)]
    static_ip: bool,
}


#[embassy_executor::task]
async fn net_task(stack: &'static Stack<TunTapDevice>) -> ! {
    stack.run().await
}

#[embassy_executor::task]
async fn main_task(spawner: Spawner) {
    let opts: Opts = Opts::parse();

    // Init network device
    let device = TunTapDevice::new(&opts.tap).unwrap();

    // Choose between dhcp or static ip
    let config = if opts.static_ip {
        Config::ipv4_static(embassy_net::StaticConfigV4 {
            address: Ipv4Cidr::new(Ipv4Address::new(192, 168, 69, 2), 24),
            dns_servers: Vec::new(),
            gateway: Some(Ipv4Address::new(192, 168, 69, 1)),
        })
    } else {
        Config::dhcpv4(Default::default())
    };

    // Generate random seed
    let mut seed = [0; 8];
    OsRng.fill_bytes(&mut seed);
    let seed = u64::from_le_bytes(seed);

    // Init network stack
    static STACK: StaticCell<Stack<TunTapDevice>> = StaticCell::new();
    static RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();
    let stack = &*STACK.init(Stack::new(
        device,
        config,
        RESOURCES.init(StackResources::<3>::new()),
        seed,
    ));

    // Launch network task
    spawner.spawn(net_task(stack)).unwrap();

    // Then we can use it!
    let mut rx_meta = [PacketMetadata::EMPTY; 16];
    let mut rx_buffer = [0; 4096];
    let mut tx_meta = [PacketMetadata::EMPTY; 16];
    let mut tx_buffer = [0; 4096];
    let mut buf = [0; 4096];

    let mut socket = UdpSocket::new(
        stack,
        &mut rx_meta,
        &mut rx_buffer,
        &mut tx_meta,
        &mut tx_buffer,
    );
    socket.bind(9400).unwrap();

    loop {
        let (n, ep) = socket.recv_from(&mut buf).await.unwrap();
        if let Ok(s) = core::str::from_utf8(&buf[..n]) {
            info!("ECHO (to {}): {}", ep, s);
        } else {
            info!("ECHO (to {}): bytearray len {}", ep, n);
        }
        socket.send_to(&buf[..n], ep).await.unwrap();
    }
}

static EXECUTOR: StaticCell<Executor> = StaticCell::new();

fn main() {
    env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .filter_module("async_io", log::LevelFilter::Info)
        .format_timestamp_nanos()
        .init();

    let executor = EXECUTOR.init(Executor::new());
    executor.run(|spawner| {
        spawner.spawn(main_task(spawner)).unwrap();
    });
}
*/
