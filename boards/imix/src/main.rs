// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2022.

//! Board file for Imix development platform.
//!
//! - <https://github.com/tock/tock/tree/master/boards/imix>
//! - <https://github.com/tock/imix>

#![no_std]
// Disable this attribute when documenting, as a workaround for
// https://github.com/rust-lang/rust/issues/62184.
#![cfg_attr(not(doc), no_main)]
#![deny(missing_docs)]

mod imix_components;
use capsules_core::alarm::AlarmDriver;
use capsules_core::console_ordered::ConsoleOrdered;
use capsules_core::virtualizers::virtual_aes_ccm::MuxAES128CCM;
use capsules_core::virtualizers::virtual_alarm::VirtualMuxAlarm;
use capsules_core::virtualizers::virtual_i2c::MuxI2C;
use capsules_core::virtualizers::virtual_spi::VirtualSpiMasterDevice;
use capsules_extra::net::ieee802154::MacAddress;
use capsules_extra::net::ipv6::ip_utils::IPAddr;
use kernel::capabilities;
use kernel::component::Component;
use kernel::deferred_call::DeferredCallClient;
use kernel::hil::digest::Digest;
use kernel::hil::i2c::I2CMaster;
use kernel::hil::radio;
#[allow(unused_imports)]
use kernel::hil::radio::{RadioConfig, RadioData};
use kernel::hil::symmetric_encryption::AES128;
use kernel::platform::{KernelResources, SyscallDriverLookup};
use kernel::process_checker::basic::AppCheckerSha256;
use kernel::scheduler::round_robin::RoundRobinSched;

//use kernel::hil::time::Alarm;
use kernel::hil::led::LedHigh;
use kernel::hil::Controller;
#[allow(unused_imports)]
use kernel::{create_capability, debug, debug_gpio, static_buf, static_init};
use sam4l::chip::Sam4lDefaultPeripherals;

use capsules_extra::sha256::Sha256Software;

use components;
use components::alarm::{AlarmDriverComponent, AlarmMuxComponent};
use components::console::{ConsoleOrderedComponent, UartMuxComponent};
use components::crc::CrcComponent;
use components::debug_writer::DebugWriterComponent;
use components::gpio::GpioComponent;
use components::isl29035::AmbientLightComponent;
use components::isl29035::Isl29035Component;
use components::led::LedsComponent;
use components::nrf51822::Nrf51822Component;
use components::process_console::ProcessConsoleComponent;
use components::rng::RngComponent;
use components::si7021::SI7021Component;
use components::spi::{SpiComponent, SpiSyscallComponent};

/// Support routines for debugging I/O.
///
/// Note: Use of this module will trample any other USART3 configuration.
pub mod io;

// Unit Tests for drivers.
#[allow(dead_code)]
mod test;

// Helper functions for enabling/disabling power on Imix submodules
mod power;

#[allow(dead_code)]
mod alarm_test;

#[allow(dead_code)]
mod multi_timer_test;

// State for loading apps.

const NUM_PROCS: usize = 4;

// Constants related to the configuration of the 15.4 network stack
// TODO: Notably, the radio MAC addresses can be configured from userland at the moment
// We probably want to change this from a security perspective (multiple apps being
// able to change the MAC address seems problematic), but it is very convenient for
// development to be able to just flash two corresponding apps onto two devices and
// have those devices talk to each other without having to modify the kernel flashed
// onto each device. This makes MAC address configuration a good target for capabilities -
// only allow one app per board to have control of MAC address configuration?
const RADIO_CHANNEL: u8 = 26;
const DST_MAC_ADDR: MacAddress = MacAddress::Short(49138);
const DEFAULT_CTX_PREFIX_LEN: u8 = 8; //Length of context for 6LoWPAN compression
const DEFAULT_CTX_PREFIX: [u8; 16] = [0x0 as u8; 16]; //Context for 6LoWPAN Compression
const PAN_ID: u16 = 0xABCD;

// how should the kernel respond when a process faults
const FAULT_RESPONSE: kernel::process::StopFaultPolicy = kernel::process::StopFaultPolicy {};

static mut PROCESSES: [Option<&'static dyn kernel::process::Process>; NUM_PROCS] =
    [None; NUM_PROCS];

static mut CHIP: Option<&'static sam4l::chip::Sam4l<Sam4lDefaultPeripherals>> = None;
static mut PROCESS_PRINTER: Option<&'static kernel::process::ProcessPrinterText> = None;

/// Dummy buffer that causes the linker to reserve enough space for the stack.
#[no_mangle]
#[link_section = ".stack_buffer"]
pub static mut STACK_MEMORY: [u8; 0x2000] = [0; 0x2000];

// Function for the process console to use to reboot the board
fn reset() -> ! {
    unsafe {
        cortexm4::scb::reset();
    }
    loop {
        cortexm4::support::nop();
    }
}

struct Imix {
    pconsole: &'static capsules_core::process_console::ProcessConsole<
        'static,
        { capsules_core::process_console::DEFAULT_COMMAND_HISTORY_LEN },
        capsules_core::virtualizers::virtual_alarm::VirtualMuxAlarm<
            'static,
            sam4l::ast::Ast<'static>,
        >,
        components::process_console::Capability,
    >,
    console: &'static capsules_core::console_ordered::ConsoleOrdered<
        'static,
        VirtualMuxAlarm<'static, sam4l::ast::Ast<'static>>,
    >,
    gpio: &'static capsules_core::gpio::GPIO<'static, sam4l::gpio::GPIOPin<'static>>,
    alarm: &'static AlarmDriver<'static, VirtualMuxAlarm<'static, sam4l::ast::Ast<'static>>>,
    temp: &'static capsules_extra::temperature::TemperatureSensor<'static>,
    humidity: &'static capsules_extra::humidity::HumiditySensor<'static>,
    ambient_light: &'static capsules_extra::ambient_light::AmbientLight<'static>,
    adc: &'static capsules_core::adc::AdcDedicated<'static, sam4l::adc::Adc<'static>>,
    led: &'static capsules_core::led::LedDriver<
        'static,
        LedHigh<'static, sam4l::gpio::GPIOPin<'static>>,
        1,
    >,
    button: &'static capsules_core::button::Button<'static, sam4l::gpio::GPIOPin<'static>>,
    rng: &'static capsules_core::rng::RngDriver<'static>,
    analog_comparator: &'static capsules_extra::analog_comparator::AnalogComparator<
        'static,
        sam4l::acifc::Acifc<'static>,
    >,
    spi: &'static capsules_core::spi_controller::Spi<
        'static,
        VirtualSpiMasterDevice<'static, sam4l::spi::SpiHw>,
    >,
    ipc: kernel::ipc::IPC<{ NUM_PROCS as u8 }>,
    ninedof: &'static capsules_extra::ninedof::NineDof<'static>,
    udp_driver: &'static capsules_extra::net::udp::UDPDriver<'static>,
    crc: &'static capsules_extra::crc::CrcDriver<'static, sam4l::crccu::Crccu<'static>>,
    usb_driver: &'static capsules_extra::usb::usb_user::UsbSyscallDriver<
        'static,
        capsules_extra::usb::usbc_client::Client<'static, sam4l::usbc::Usbc<'static>>,
    >,
    nrf51822: &'static capsules_extra::nrf51822_serialization::Nrf51822Serialization<'static>,
    nonvolatile_storage:
        &'static capsules_extra::nonvolatile_storage_driver::NonvolatileStorage<'static>,
    scheduler: &'static RoundRobinSched<'static>,
    systick: cortexm4::systick::SysTick,
    credentials_checking_policy: &'static (),
    //credentials_checking_policy: &'static AppCheckerSha256,
}

// The RF233 radio stack requires our buffers for its SPI operations:
//
//   1. buf: a packet-sized buffer for SPI operations, which is
//      used as the read buffer when it writes a packet passed to it and the write
//      buffer when it reads a packet into a buffer passed to it.
//   2. rx_buf: buffer to receive packets into
//   3 + 4: two small buffers for performing registers
//      operations (one read, one write).

static mut RF233_BUF: [u8; radio::MAX_BUF_SIZE] = [0x00; radio::MAX_BUF_SIZE];
static mut RF233_REG_WRITE: [u8; 2] = [0x00; 2];
static mut RF233_REG_READ: [u8; 2] = [0x00; 2];
static mut SHA256_CHECKER_BUF: [u8; 32] = [0; 32];

impl SyscallDriverLookup for Imix {
    fn with_driver<F, R>(&self, driver_num: usize, f: F) -> R
    where
        F: FnOnce(Option<&dyn kernel::syscall::SyscallDriver>) -> R,
    {
        match driver_num {
            capsules_core::console_ordered::DRIVER_NUM => f(Some(self.console)),
            capsules_core::gpio::DRIVER_NUM => f(Some(self.gpio)),
            capsules_core::alarm::DRIVER_NUM => f(Some(self.alarm)),
            capsules_core::spi_controller::DRIVER_NUM => f(Some(self.spi)),
            capsules_core::adc::DRIVER_NUM => f(Some(self.adc)),
            capsules_core::led::DRIVER_NUM => f(Some(self.led)),
            capsules_core::button::DRIVER_NUM => f(Some(self.button)),
            capsules_extra::analog_comparator::DRIVER_NUM => f(Some(self.analog_comparator)),
            capsules_extra::ambient_light::DRIVER_NUM => f(Some(self.ambient_light)),
            capsules_extra::temperature::DRIVER_NUM => f(Some(self.temp)),
            capsules_extra::humidity::DRIVER_NUM => f(Some(self.humidity)),
            capsules_extra::ninedof::DRIVER_NUM => f(Some(self.ninedof)),
            capsules_extra::crc::DRIVER_NUM => f(Some(self.crc)),
            capsules_extra::usb::usb_user::DRIVER_NUM => f(Some(self.usb_driver)),
            capsules_extra::net::udp::DRIVER_NUM => f(Some(self.udp_driver)),
            capsules_extra::nrf51822_serialization::DRIVER_NUM => f(Some(self.nrf51822)),
            capsules_extra::nonvolatile_storage_driver::DRIVER_NUM => {
                f(Some(self.nonvolatile_storage))
            }
            capsules_core::rng::DRIVER_NUM => f(Some(self.rng)),
            kernel::ipc::DRIVER_NUM => f(Some(&self.ipc)),
            _ => f(None),
        }
    }
}

impl KernelResources<sam4l::chip::Sam4l<Sam4lDefaultPeripherals>> for Imix {
    type SyscallDriverLookup = Self;
    type SyscallFilter = ();
    type ProcessFault = ();
    type CredentialsCheckingPolicy = ();
    //type CredentialsCheckingPolicy = AppCheckerSha256;
    type Scheduler = RoundRobinSched<'static>;
    type SchedulerTimer = cortexm4::systick::SysTick;
    type WatchDog = ();
    type ContextSwitchCallback = ();

    fn syscall_driver_lookup(&self) -> &Self::SyscallDriverLookup {
        &self
    }
    fn syscall_filter(&self) -> &Self::SyscallFilter {
        &()
    }
    fn process_fault(&self) -> &Self::ProcessFault {
        &()
    }
    fn credentials_checking_policy(&self) -> &'static Self::CredentialsCheckingPolicy {
        self.credentials_checking_policy
    }
    fn scheduler(&self) -> &Self::Scheduler {
        self.scheduler
    }
    fn scheduler_timer(&self) -> &Self::SchedulerTimer {
        &self.systick
    }
    fn watchdog(&self) -> &Self::WatchDog {
        &()
    }
    fn context_switch_callback(&self) -> &Self::ContextSwitchCallback {
        &()
    }
}

unsafe fn set_pin_primary_functions(peripherals: &Sam4lDefaultPeripherals) {
    use sam4l::gpio::PeripheralFunction::{A, B, C, E};

    // Right column: Imix pin name
    // Left  column: SAM4L peripheral function
    peripherals.pa[04].configure(Some(A)); // AD0         --  ADCIFE AD0
    peripherals.pa[05].configure(Some(A)); // AD1         --  ADCIFE AD1
    peripherals.pa[06].configure(Some(C)); // EXTINT1     --  EIC EXTINT1
    peripherals.pa[07].configure(Some(A)); // AD1         --  ADCIFE AD2
    peripherals.pa[08].configure(None); //... RF233 IRQ   --  GPIO pin
    peripherals.pa[09].configure(None); //... RF233 RST   --  GPIO pin
    peripherals.pa[10].configure(None); //... RF233 SLP   --  GPIO pin
    peripherals.pa[13].configure(None); //... TRNG EN     --  GPIO pin
    peripherals.pa[14].configure(None); //... TRNG_OUT    --  GPIO pin
    peripherals.pa[17].configure(None); //... NRF INT     -- GPIO pin
    peripherals.pa[18].configure(Some(A)); // NRF CLK     -- USART2_CLK
    peripherals.pa[20].configure(None); //... D8          -- GPIO pin
    peripherals.pa[21].configure(Some(E)); // TWI2 SDA    -- TWIM2_SDA
    peripherals.pa[22].configure(Some(E)); // TWI2 SCL    --  TWIM2 TWCK
    peripherals.pa[25].configure(Some(A)); // USB_N       --  USB DM
    peripherals.pa[26].configure(Some(A)); // USB_P       --  USB DP
    peripherals.pb[00].configure(Some(A)); // TWI1_SDA    --  TWIMS1 TWD
    peripherals.pb[01].configure(Some(A)); // TWI1_SCL    --  TWIMS1 TWCK
    peripherals.pb[02].configure(Some(A)); // AD3         --  ADCIFE AD3
    peripherals.pb[03].configure(Some(A)); // AD4         --  ADCIFE AD4
    peripherals.pb[04].configure(Some(A)); // AD5         --  ADCIFE AD5
    peripherals.pb[05].configure(Some(A)); // VHIGHSAMPLE --  ADCIFE AD6
    peripherals.pb[06].configure(Some(A)); // RTS3        --  USART3 RTS
    peripherals.pb[07].configure(None); //... NRF RESET   --  GPIO
    peripherals.pb[09].configure(Some(A)); // RX3         --  USART3 RX
    peripherals.pb[10].configure(Some(A)); // TX3         --  USART3 TX
    peripherals.pb[11].configure(Some(A)); // CTS0        --  USART0 CTS
    peripherals.pb[12].configure(Some(A)); // RTS0        --  USART0 RTS
    peripherals.pb[13].configure(Some(A)); // CLK0        --  USART0 CLK
    peripherals.pb[14].configure(Some(A)); // RX0         --  USART0 RX
    peripherals.pb[15].configure(Some(A)); // TX0         --  USART0 TX
    peripherals.pc[00].configure(Some(A)); // CS2         --  SPI Nperipherals.pcS2
    peripherals.pc[01].configure(Some(A)); // CS3 (RF233) --  SPI Nperipherals.pcS3
    peripherals.pc[02].configure(Some(A)); // CS1         --  SPI Nperipherals.pcS1
    peripherals.pc[03].configure(Some(A)); // CS0         --  SPI Nperipherals.pcS0
    peripherals.pc[04].configure(Some(A)); // MISO        --  SPI MISO
    peripherals.pc[05].configure(Some(A)); // MOSI        --  SPI MOSI
    peripherals.pc[06].configure(Some(A)); // SCK         --  SPI CLK
    peripherals.pc[07].configure(Some(B)); // RTS2 (BLE)  -- USART2_RTS
    peripherals.pc[08].configure(Some(E)); // CTS2 (BLE)  -- USART2_CTS
                                           //peripherals.pc[09].configure(None); //... NRF GPIO    -- GPIO
                                           //peripherals.pc[10].configure(None); //... USER LED    -- GPIO
    peripherals.pc[09].configure(Some(E)); // ACAN1       -- ACIFC comparator
    peripherals.pc[10].configure(Some(E)); // ACAP1       -- ACIFC comparator
    peripherals.pc[11].configure(Some(B)); // RX2 (BLE)   -- USART2_RX
    peripherals.pc[12].configure(Some(B)); // TX2 (BLE)   -- USART2_TX
                                           //peripherals.pc[13].configure(None); //... ACC_INT1    -- GPIO
                                           //peripherals.pc[14].configure(None); //... ACC_INT2    -- GPIO
    peripherals.pc[13].configure(Some(E)); //... ACBN1    -- ACIFC comparator
    peripherals.pc[14].configure(Some(E)); //... ACBP1    -- ACIFC comparator
    peripherals.pc[16].configure(None); //... SENSE_PWR   --  GPIO pin
    peripherals.pc[17].configure(None); //... NRF_PWR     --  GPIO pin
    peripherals.pc[18].configure(None); //... RF233_PWR   --  GPIO pin
    peripherals.pc[19].configure(None); //... TRNG_PWR    -- GPIO Pin
    peripherals.pc[22].configure(None); //... KERNEL LED  -- GPIO Pin
    peripherals.pc[24].configure(None); //... USER_BTN    -- GPIO Pin
    peripherals.pc[25].configure(Some(B)); // LI_INT      --  EIC EXTINT2
    peripherals.pc[26].configure(None); //... D7          -- GPIO Pin
    peripherals.pc[27].configure(None); //... D6          -- GPIO Pin
    peripherals.pc[28].configure(None); //... D5          -- GPIO Pin
    peripherals.pc[29].configure(None); //... D4          -- GPIO Pin
    peripherals.pc[30].configure(None); //... D3          -- GPIO Pin
    peripherals.pc[31].configure(None); //... D2          -- GPIO Pin
}

/// This is in a separate, inline(never) function so that its stack frame is
/// removed when this function returns. Otherwise, the stack space used for
/// these static_inits is wasted.
#[inline(never)]
unsafe fn create_peripherals(
    pm: &'static sam4l::pm::PowerManager,
) -> &'static Sam4lDefaultPeripherals {
    static_init!(Sam4lDefaultPeripherals, Sam4lDefaultPeripherals::new(pm))
}

/// Main function.
///
/// This is called after RAM initialization is complete.
#[no_mangle]
pub unsafe fn main() {
    sam4l::init();
    let pm = static_init!(sam4l::pm::PowerManager, sam4l::pm::PowerManager::new());
    let peripherals = create_peripherals(pm);

    pm.setup_system_clock(
        sam4l::pm::SystemClockSource::PllExternalOscillatorAt48MHz {
            frequency: sam4l::pm::OscillatorFrequency::Frequency16MHz,
            startup_mode: sam4l::pm::OscillatorStartup::FastStart,
        },
        &peripherals.flash_controller,
    );

    // Source 32Khz and 1Khz clocks from RC23K (SAM4L Datasheet 11.6.8)
    sam4l::bpm::set_ck32source(sam4l::bpm::CK32Source::RC32K);

    set_pin_primary_functions(peripherals);

    peripherals.setup_circular_deps();
    let chip = static_init!(
        sam4l::chip::Sam4l<Sam4lDefaultPeripherals>,
        sam4l::chip::Sam4l::new(pm, peripherals)
    );
    CHIP = Some(chip);

    // Create capabilities that the board needs to call certain protected kernel
    // functions.
    let process_mgmt_cap = create_capability!(capabilities::ProcessManagementCapability);
    let main_cap = create_capability!(capabilities::MainLoopCapability);
    let grant_cap = create_capability!(capabilities::MemoryAllocationCapability);

    power::configure_submodules(
        &peripherals.pa,
        &peripherals.pb,
        &peripherals.pc,
        power::SubmoduleConfig {
            rf233: true,
            nrf51422: true,
            sensors: true,
            trng: true,
        },
    );

    let sha = static_init!(Sha256Software<'static>, Sha256Software::new());
    kernel::deferred_call::DeferredCallClient::register(sha);

    let checker = static_init!(
        AppCheckerSha256,
        AppCheckerSha256::new(sha, &mut SHA256_CHECKER_BUF)
    );
    sha.set_client(checker);

    let board_kernel = static_init!(kernel::Kernel, kernel::Kernel::new(&PROCESSES));

    let process_printer = components::process_printer::ProcessPrinterTextComponent::new()
        .finalize(components::process_printer_text_component_static!());
    PROCESS_PRINTER = Some(process_printer);

    // # CONSOLE
    // Create a shared UART channel for the consoles and for kernel debug.
    peripherals.usart3.set_mode(sam4l::usart::UsartMode::Uart);
    let uart_mux = UartMuxComponent::new(&peripherals.usart3, 115200)
        .finalize(components::uart_mux_component_static!());

    // # TIMER
    let mux_alarm = AlarmMuxComponent::new(&peripherals.ast)
        .finalize(components::alarm_mux_component_static!(sam4l::ast::Ast));
    peripherals.ast.configure(mux_alarm);

    let alarm =
        AlarmDriverComponent::new(board_kernel, capsules_core::alarm::DRIVER_NUM, mux_alarm)
            .finalize(components::alarm_component_static!(sam4l::ast::Ast));

    let pconsole = ProcessConsoleComponent::new(
        board_kernel,
        uart_mux,
        mux_alarm,
        process_printer,
        Some(reset),
    )
    .finalize(components::process_console_component_static!(
        sam4l::ast::Ast
    ));

    let console = ConsoleOrderedComponent::new(
        board_kernel,
        capsules_core::console_ordered::DRIVER_NUM,
        uart_mux,
        mux_alarm,
        200,
        5,
        5,
    )
    .finalize(components::console_ordered_component_static!(
        sam4l::ast::Ast
    ));
    DebugWriterComponent::new(uart_mux).finalize(components::debug_writer_component_static!());

    // Allow processes to communicate over BLE through the nRF51822
    peripherals.usart2.set_mode(sam4l::usart::UsartMode::Uart);
    let nrf_serialization = Nrf51822Component::new(
        board_kernel,
        capsules_extra::nrf51822_serialization::DRIVER_NUM,
        &peripherals.usart2,
        &peripherals.pb[07],
    )
    .finalize(components::nrf51822_component_static!());

    // # I2C and I2C Sensors
    let mux_i2c = static_init!(
        MuxI2C<'static, sam4l::i2c::I2CHw>,
        MuxI2C::new(&peripherals.i2c2, None)
    );
    kernel::deferred_call::DeferredCallClient::register(mux_i2c);
    peripherals.i2c2.set_master_client(mux_i2c);

    let isl29035 = Isl29035Component::new(mux_i2c, mux_alarm).finalize(
        components::isl29035_component_static!(sam4l::ast::Ast, sam4l::i2c::I2CHw),
    );
    let ambient_light = AmbientLightComponent::new(
        board_kernel,
        capsules_extra::ambient_light::DRIVER_NUM,
        isl29035,
    )
    .finalize(components::ambient_light_component_static!());

    let si7021 = SI7021Component::new(mux_i2c, mux_alarm, 0x40).finalize(
        components::si7021_component_static!(sam4l::ast::Ast, sam4l::i2c::I2CHw),
    );
    let temp = components::temperature::TemperatureComponent::new(
        board_kernel,
        capsules_extra::temperature::DRIVER_NUM,
        si7021,
    )
    .finalize(components::temperature_component_static!());
    let humidity = components::humidity::HumidityComponent::new(
        board_kernel,
        capsules_extra::humidity::DRIVER_NUM,
        si7021,
    )
    .finalize(components::humidity_component_static!());

    let fxos8700 = components::fxos8700::Fxos8700Component::new(mux_i2c, 0x1e, &peripherals.pc[13])
        .finalize(components::fxos8700_component_static!(sam4l::i2c::I2CHw));

    let ninedof = components::ninedof::NineDofComponent::new(
        board_kernel,
        capsules_extra::ninedof::DRIVER_NUM,
    )
    .finalize(components::ninedof_component_static!(fxos8700));

    // SPI MUX, SPI syscall driver and RF233 radio
    let mux_spi = components::spi::SpiMuxComponent::new(&peripherals.spi)
        .finalize(components::spi_mux_component_static!(sam4l::spi::SpiHw));

    let spi_syscalls = SpiSyscallComponent::new(
        board_kernel,
        mux_spi,
        2,
        capsules_core::spi_controller::DRIVER_NUM,
    )
    .finalize(components::spi_syscall_component_static!(sam4l::spi::SpiHw));
    let rf233_spi = SpiComponent::new(mux_spi, 3)
        .finalize(components::spi_component_static!(sam4l::spi::SpiHw));
    let rf233 = components::rf233::RF233Component::new(
        rf233_spi,
        &peripherals.pa[09], // reset
        &peripherals.pa[10], // sleep
        &peripherals.pa[08], // irq
        &peripherals.pa[08],
        RADIO_CHANNEL,
    )
    .finalize(components::rf233_component_static!(sam4l::spi::SpiHw));

    // Setup ADC
    let adc_channels = static_init!(
        [sam4l::adc::AdcChannel; 6],
        [
            sam4l::adc::AdcChannel::new(sam4l::adc::Channel::AD1), // AD0
            sam4l::adc::AdcChannel::new(sam4l::adc::Channel::AD2), // AD1
            sam4l::adc::AdcChannel::new(sam4l::adc::Channel::AD3), // AD2
            sam4l::adc::AdcChannel::new(sam4l::adc::Channel::AD4), // AD3
            sam4l::adc::AdcChannel::new(sam4l::adc::Channel::AD5), // AD4
            sam4l::adc::AdcChannel::new(sam4l::adc::Channel::AD6), // AD5
        ]
    );
    let adc = components::adc::AdcDedicatedComponent::new(
        &peripherals.adc,
        adc_channels,
        board_kernel,
        capsules_core::adc::DRIVER_NUM,
    )
    .finalize(components::adc_dedicated_component_static!(sam4l::adc::Adc));

    let gpio = GpioComponent::new(
        board_kernel,
        capsules_core::gpio::DRIVER_NUM,
        components::gpio_component_helper!(
            sam4l::gpio::GPIOPin,
            0 => &peripherals.pc[31],
            1 => &peripherals.pc[30],
            2 => &peripherals.pc[29],
            3 => &peripherals.pc[28],
            4 => &peripherals.pc[27],
            5 => &peripherals.pc[26],
            6 => &peripherals.pa[20]
        ),
    )
    .finalize(components::gpio_component_static!(sam4l::gpio::GPIOPin));

    let led = LedsComponent::new().finalize(components::led_component_static!(
        LedHigh<'static, sam4l::gpio::GPIOPin>,
        LedHigh::new(&peripherals.pc[10]),
    ));

    let button = components::button::ButtonComponent::new(
        board_kernel,
        capsules_core::button::DRIVER_NUM,
        components::button_component_helper!(
            sam4l::gpio::GPIOPin,
            (
                &peripherals.pc[24],
                kernel::hil::gpio::ActivationMode::ActiveLow,
                kernel::hil::gpio::FloatingState::PullNone
            )
        ),
    )
    .finalize(components::button_component_static!(sam4l::gpio::GPIOPin));

    let crc = CrcComponent::new(
        board_kernel,
        capsules_extra::crc::DRIVER_NUM,
        &peripherals.crccu,
    )
    .finalize(components::crc_component_static!(sam4l::crccu::Crccu));

    let ac_0 = static_init!(
        sam4l::acifc::AcChannel,
        sam4l::acifc::AcChannel::new(sam4l::acifc::Channel::AC0)
    );
    let ac_1 = static_init!(
        sam4l::acifc::AcChannel,
        sam4l::acifc::AcChannel::new(sam4l::acifc::Channel::AC0)
    );
    let ac_2 = static_init!(
        sam4l::acifc::AcChannel,
        sam4l::acifc::AcChannel::new(sam4l::acifc::Channel::AC0)
    );
    let ac_3 = static_init!(
        sam4l::acifc::AcChannel,
        sam4l::acifc::AcChannel::new(sam4l::acifc::Channel::AC0)
    );
    let analog_comparator = components::analog_comparator::AnalogComparatorComponent::new(
        &peripherals.acifc,
        components::analog_comparator_component_helper!(
            <sam4l::acifc::Acifc as kernel::hil::analog_comparator::AnalogComparator>::Channel,
            ac_0,
            ac_1,
            ac_2,
            ac_3
        ),
        board_kernel,
        capsules_extra::analog_comparator::DRIVER_NUM,
    )
    .finalize(components::analog_comparator_component_static!(
        sam4l::acifc::Acifc
    ));
    let rng = RngComponent::new(
        board_kernel,
        capsules_core::rng::DRIVER_NUM,
        &peripherals.trng,
    )
    .finalize(components::rng_component_static!());

    // For now, assign the 802.15.4 MAC address on the device as
    // simply a 16-bit short address which represents the last 16 bits
    // of the serial number of the sam4l for this device.  In the
    // future, we could generate the MAC address by hashing the full
    // 120-bit serial number
    let serial_num: sam4l::serial_num::SerialNum = sam4l::serial_num::SerialNum::new();
    let serial_num_bottom_16 = (serial_num.get_lower_64() & 0x0000_0000_0000_ffff) as u16;
    let src_mac_from_serial_num: MacAddress = MacAddress::Short(serial_num_bottom_16);

    let aes_mux = static_init!(
        MuxAES128CCM<'static, sam4l::aes::Aes>,
        MuxAES128CCM::new(&peripherals.aes)
    );
    aes_mux.register();
    peripherals.aes.set_client(aes_mux);

    // Can this initialize be pushed earlier, or into component? -pal
    let _ = rf233.initialize(&mut RF233_BUF, &mut RF233_REG_WRITE, &mut RF233_REG_READ);
    let (_, mux_mac) = components::ieee802154::Ieee802154Component::new(
        board_kernel,
        capsules_extra::ieee802154::DRIVER_NUM,
        rf233,
        aes_mux,
        PAN_ID,
        serial_num_bottom_16,
    )
    .finalize(components::ieee802154_component_static!(
        capsules_extra::rf233::RF233<'static, VirtualSpiMasterDevice<'static, sam4l::spi::SpiHw>>,
        sam4l::aes::Aes<'static>
    ));

    let usb_driver = components::usb::UsbComponent::new(
        board_kernel,
        capsules_extra::usb::usb_user::DRIVER_NUM,
        &peripherals.usbc,
    )
    .finalize(components::usb_component_static!(sam4l::usbc::Usbc));

    // Kernel storage region, allocated with the storage_volume!
    // macro in common/utils.rs
    extern "C" {
        /// Beginning on the ROM region containing app images.
        static _sstorage: u8;
        static _estorage: u8;
    }

    let nonvolatile_storage = components::nonvolatile_storage::NonvolatileStorageComponent::new(
        board_kernel,
        capsules_extra::nonvolatile_storage_driver::DRIVER_NUM,
        &peripherals.flash_controller,
        0x60000,                          // Start address for userspace accessible region
        0x20000,                          // Length of userspace accessible region
        &_sstorage as *const u8 as usize, //start address of kernel region
        &_estorage as *const u8 as usize - &_sstorage as *const u8 as usize, // length of kernel region
    )
    .finalize(components::nonvolatile_storage_component_static!(
        sam4l::flashcalw::FLASHCALW
    ));

    let local_ip_ifaces = static_init!(
        [IPAddr; 3],
        [
            IPAddr([
                0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
                0x0e, 0x0f,
            ]),
            IPAddr([
                0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d,
                0x1e, 0x1f,
            ]),
            IPAddr::generate_from_mac(src_mac_from_serial_num),
        ]
    );

    let (udp_send_mux, udp_recv_mux, udp_port_table) = components::udp_mux::UDPMuxComponent::new(
        mux_mac,
        DEFAULT_CTX_PREFIX_LEN,
        DEFAULT_CTX_PREFIX,
        DST_MAC_ADDR,
        src_mac_from_serial_num, //comment out for dual rx test only
        //MacAddress::Short(49138), //comment in for dual rx test only
        local_ip_ifaces,
        mux_alarm,
    )
    .finalize(components::udp_mux_component_static!(sam4l::ast::Ast));

    // UDP driver initialization happens here
    let udp_driver = components::udp_driver::UDPDriverComponent::new(
        board_kernel,
        capsules_extra::net::udp::driver::DRIVER_NUM,
        udp_send_mux,
        udp_recv_mux,
        udp_port_table,
        local_ip_ifaces,
    )
    .finalize(components::udp_driver_component_static!(sam4l::ast::Ast));

    let scheduler = components::sched::round_robin::RoundRobinComponent::new(&PROCESSES)
        .finalize(components::round_robin_component_static!(NUM_PROCS));

    let imix = Imix {
        pconsole,
        console,
        alarm,
        gpio,
        temp,
        humidity,
        ambient_light,
        adc,
        led,
        button,
        rng,
        analog_comparator,
        crc,
        spi: spi_syscalls,
        ipc: kernel::ipc::IPC::new(board_kernel, kernel::ipc::DRIVER_NUM, &grant_cap),
        ninedof,
        udp_driver,
        usb_driver,
        nrf51822: nrf_serialization,
        nonvolatile_storage,
        scheduler,
        systick: cortexm4::systick::SysTick::new(),
        //credentials_checking_policy: checker,
        credentials_checking_policy: &(),
    };

    // Need to initialize the UART for the nRF51 serialization.
    imix.nrf51822.initialize();

    // These two lines need to be below the creation of the chip for
    // initialization to work.
    let _ = rf233.reset();
    let _ = rf233.start();

    let _ = imix.pconsole.start();

    // Optional kernel tests. Note that these might conflict
    // with normal operation (e.g., steal callbacks from drivers, etc.),
    // so do not run these and expect all services/applications to work.
    // Once everything is virtualized in the kernel this won't be a problem.
    // -pal, 11/20/18
    //
    //test::virtual_uart_rx_test::run_virtual_uart_receive(uart_mux);
    //test::rng_test::run_entropy32(&peripherals.trng);
    //test::virtual_aes_ccm_test::run(&peripherals.aes);
    //test::aes_test::run_aes128_ctr(&peripherals.aes);
    //test::aes_test::run_aes128_cbc(&peripherals.aes);
    //test::log_test::run(
    //    mux_alarm,
    //    &peripherals.flash_controller,
    //);
    //test::linear_log_test::run(
    //    mux_alarm,
    //    &peripherals.flash_controller,
    //);
    //test::icmp_lowpan_test::run(mux_mac, mux_alarm);
    //let lowpan_frag_test = test::ipv6_lowpan_test::initialize_all(mux_mac, mux_alarm);
    //lowpan_frag_test.start(); // If flashing the transmitting Imix
    /*let udp_lowpan_test = test::udp_lowpan_test::initialize_all(
       udp_send_mux,
        udp_recv_mux,
        udp_port_table,
        mux_alarm,
    );*/
    //udp_lowpan_test.start();

    // alarm_test::run_alarm(&peripherals.ast);
    /*let virtual_alarm_timer = static_init!(
        VirtualMuxAlarm<'static, sam4l::ast::Ast>,
        VirtualMuxAlarm::new(mux_alarm)
    );
    virtual_alarm_timer.setup();

    let mux_timer = static_init!(
        MuxTimer<'static, sam4l::ast::Ast>,
        MuxTimer::new(virtual_alarm_timer)
    );*/
    //virtual_alarm_timer.set_alarm_client(mux_timer);

    //test::sha256_test::run_sha256();

    /*components::test::multi_alarm_test::MultiAlarmTestComponent::new(mux_alarm)
    .finalize(components::multi_alarm_test_component_buf!(sam4l::ast::Ast))
    .run();*/

    debug!("Initialization complete. Entering main loop");

    // These symbols are defined in the linker script.
    extern "C" {
        /// Beginning of the ROM region containing app images.
        static _sapps: u8;
        /// End of the ROM region containing app images.
        static _eapps: u8;
        /// Beginning of the RAM region for app memory.
        static mut _sappmem: u8;
        /// End of the RAM region for app memory.
        static _eappmem: u8;
    }

    kernel::process::load_and_check_processes(
        board_kernel,
        &imix,
        chip,
        core::slice::from_raw_parts(
            &_sapps as *const u8,
            &_eapps as *const u8 as usize - &_sapps as *const u8 as usize,
        ),
        core::slice::from_raw_parts_mut(
            &mut _sappmem as *mut u8,
            &_eappmem as *const u8 as usize - &_sappmem as *const u8 as usize,
        ),
        &mut PROCESSES,
        &FAULT_RESPONSE,
        &process_mgmt_cap,
    )
    .unwrap_or_else(|err| {
        debug!("Error loading processes!");
        debug!("{:?}", err);
    });

    board_kernel.kernel_loop(&imix, chip, Some(&imix.ipc), &main_cap);
}
