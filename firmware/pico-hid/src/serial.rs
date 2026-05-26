use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::peripherals::USB;
use embassy_rp::usb::{Driver, InterruptHandler};
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
use embassy_usb::driver::EndpointError;
use embassy_usb::{Builder, Config, UsbDevice};
use static_cell::StaticCell;

use crate::usb;

const CDC_MAX_PACKET_SIZE: u16 = 64;
const CONFIG_DESCRIPTOR_LEN: usize = 256;
const BOS_DESCRIPTOR_LEN: usize = 256;
const CONTROL_BUF_LEN: usize = 64;

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => InterruptHandler<USB>;
});

type UsbDriver = Driver<'static, USB>;
type PicoUsbDevice = UsbDevice<'static, UsbDriver>;
type PicoCdcAcmClass = CdcAcmClass<'static, UsbDriver>;

pub fn spawn_usb(usb_peripheral: embassy_rp::Peri<'static, USB>, spawner: &Spawner) -> bool {
    let (usb_device, serial_class) = build_usb_serial(usb_peripheral);

    let device_spawned = match usb_task(usb_device) {
        Ok(token) => {
            spawner.spawn(token);
            true
        }
        Err(_) => false,
    };
    let echo_spawned = match cdc_echo_task(serial_class) {
        Ok(token) => {
            spawner.spawn(token);
            true
        }
        Err(_) => false,
    };

    device_spawned && echo_spawned
}

fn build_usb_serial(
    usb_peripheral: embassy_rp::Peri<'static, USB>,
) -> (PicoUsbDevice, PicoCdcAcmClass) {
    let driver = Driver::new(usb_peripheral, Irqs);
    let identity = usb::identity();

    let mut config = Config::new(identity.vid, identity.pid);
    config.manufacturer = Some(identity.manufacturer);
    config.product = Some(identity.product);
    config.serial_number = Some(identity.serial_prefix);
    config.max_power = 100;
    config.max_packet_size_0 = 64;

    static CONFIG_DESCRIPTOR: StaticCell<[u8; CONFIG_DESCRIPTOR_LEN]> = StaticCell::new();
    static BOS_DESCRIPTOR: StaticCell<[u8; BOS_DESCRIPTOR_LEN]> = StaticCell::new();
    static CONTROL_BUF: StaticCell<[u8; CONTROL_BUF_LEN]> = StaticCell::new();
    static STATE: StaticCell<State<'static>> = StaticCell::new();

    let mut builder = Builder::new(
        driver,
        config,
        CONFIG_DESCRIPTOR.init([0; CONFIG_DESCRIPTOR_LEN]),
        BOS_DESCRIPTOR.init([0; BOS_DESCRIPTOR_LEN]),
        &mut [],
        CONTROL_BUF.init([0; CONTROL_BUF_LEN]),
    );

    let state = STATE.init(State::new());
    let serial_class = CdcAcmClass::new(&mut builder, state, CDC_MAX_PACKET_SIZE);
    let usb_device = builder.build();

    (usb_device, serial_class)
}

#[embassy_executor::task]
async fn usb_task(mut usb: PicoUsbDevice) -> ! {
    usb.run().await
}

#[embassy_executor::task]
async fn cdc_echo_task(mut serial_class: PicoCdcAcmClass) -> ! {
    loop {
        serial_class.wait_connection().await;
        let _disconnected = echo_until_disconnect(&mut serial_class).await;
    }
}

async fn echo_until_disconnect(serial_class: &mut PicoCdcAcmClass) -> Result<(), Disconnected> {
    let mut packet = [0u8; CDC_MAX_PACKET_SIZE as usize];

    loop {
        let count = serial_class.read_packet(&mut packet).await?;
        let data = &packet[..count];

        serial_class.write_packet(data).await?;
        if count == serial_class.max_packet_size() as usize {
            serial_class.write_packet(&[]).await?;
        }
    }
}

struct Disconnected;

impl From<EndpointError> for Disconnected {
    fn from(value: EndpointError) -> Self {
        match value {
            EndpointError::BufferOverflow | EndpointError::Disabled => Self,
        }
    }
}
