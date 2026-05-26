#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_rp::gpio::{Level, Output};
use embassy_time::Timer;

mod serial;
mod usb;

#[cfg(feature = "defmt")]
use defmt::info;
#[cfg(not(feature = "defmt"))]
use panic_halt as _;
#[cfg(feature = "defmt")]
use {defmt_rtt as _, panic_probe as _};

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let peripherals = embassy_rp::init(Default::default());
    let usb_ready = serial::spawn_usb(peripherals.USB, &spawner);
    let mut led = Output::new(peripherals.PIN_25, Level::Low);

    loop {
        #[cfg(feature = "defmt")]
        info!("gp25 led on");
        led.set_high();
        Timer::after_secs(1).await;

        #[cfg(feature = "defmt")]
        info!("gp25 led off");
        led.set_low();
        if usb_ready {
            Timer::after_secs(1).await;
        } else {
            Timer::after_millis(100).await;
        }
    }
}
