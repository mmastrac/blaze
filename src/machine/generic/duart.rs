use std::sync::mpsc;
use std::{cell::Cell, rc::Rc};

use tracing::{trace, warn};

/// Slow down ticks to allow XON/XOFF to take effect
const DUART_COOLDOWN_TICKS: u16 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ReadRegister {
    ModeRegisterA,
    StatusRegisterA,
    BrgExtend,
    RxHoldingRegisterA,
    InputPortChangeRegister,
    InterruptStatusRegister,
    CounterTimerUpperValue,
    CounterTimerLowerValue,
    ModeRegisterB,
    StatusRegisterB,
    Test1x16x,
    RxHoldingRegisterB,
    ScratchPad,
    InputPortsIP0ToIP6,
    StartCounterCommand,
    StopCounterCommand,
}

impl TryFrom<u8> for ReadRegister {
    type Error = ();
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(ReadRegister::ModeRegisterA),
            1 => Ok(ReadRegister::StatusRegisterA),
            2 => Ok(ReadRegister::BrgExtend),
            3 => Ok(ReadRegister::RxHoldingRegisterA),
            4 => Ok(ReadRegister::InputPortChangeRegister),
            5 => Ok(ReadRegister::InterruptStatusRegister),
            6 => Ok(ReadRegister::CounterTimerUpperValue),
            7 => Ok(ReadRegister::CounterTimerLowerValue),
            8 => Ok(ReadRegister::ModeRegisterB),
            9 => Ok(ReadRegister::StatusRegisterB),
            10 => Ok(ReadRegister::Test1x16x),
            11 => Ok(ReadRegister::RxHoldingRegisterB),
            12 => Ok(ReadRegister::ScratchPad),
            13 => Ok(ReadRegister::InputPortsIP0ToIP6),
            14 => Ok(ReadRegister::StartCounterCommand),
            15 => Ok(ReadRegister::StopCounterCommand),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WriteRegister {
    ModeRegisterA,
    ClockSelectRegisterA,
    CommandRegisterA,
    TxHoldingRegisterA,
    AuxControlRegister,
    InterruptMaskRegister,
    CounterTimerUpperPreset,
    CounterTimerLowerPreset,
    ModeRegisterB,
    ClockSelectRegisterB,
    CommandRegisterB,
    TxHoldingRegisterB,
    ScratchPad,
    InputPortConfRegister,
    SetOutputPortBitsCommand,
    ResetOutputPortBitsCommand,
}

impl TryFrom<u8> for WriteRegister {
    type Error = ();
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(WriteRegister::ModeRegisterA),
            1 => Ok(WriteRegister::ClockSelectRegisterA),
            2 => Ok(WriteRegister::CommandRegisterA),
            3 => Ok(WriteRegister::TxHoldingRegisterA),
            4 => Ok(WriteRegister::AuxControlRegister),
            5 => Ok(WriteRegister::InterruptMaskRegister),
            6 => Ok(WriteRegister::CounterTimerUpperPreset),
            7 => Ok(WriteRegister::CounterTimerLowerPreset),
            8 => Ok(WriteRegister::ModeRegisterB),
            9 => Ok(WriteRegister::ClockSelectRegisterB),
            10 => Ok(WriteRegister::CommandRegisterB),
            11 => Ok(WriteRegister::TxHoldingRegisterB),
            12 => Ok(WriteRegister::ScratchPad),
            13 => Ok(WriteRegister::InputPortConfRegister),
            14 => Ok(WriteRegister::SetOutputPortBitsCommand),
            15 => Ok(WriteRegister::ResetOutputPortBitsCommand),
            _ => Err(()),
        }
    }
}

const READ_2681: &[&str] = &[
    "Mode Register A (MR1A, MR2A)",
    "Status Register A (SRA)",
    "BRG Extend",
    "Rx Holding Register A (RHRA)",
    "Input Port Change Register (IPCR)",
    "Interrupt Status Register (ISR)",
    "Counter/Timer Upper Value (CTU)",
    "Counter/Timer Lower Value (CTL)",
    "Mode Register B (MR1B, MR2B)",
    "Status Register B (SRB)",
    "1x/16x Test",
    "Rx Holding Register B (RHRB)",
    "Use for scratch pad",
    "Input Ports IP0 to IP6",
    "Start Counter Command",
    "Stop Counter Command",
];

const WRITE_2681: &[&str] = &[
    "Mode Register A (MR1A, MR2A)",
    "Clock Select Register A (CSRA)",
    "Command Register A (CRA)",
    "Tx Holding Register A (THRA)",
    "Aux. Control Register (ACR)",
    "Interrupt Mask Register (IMR)",
    "C/T Upper Preset Value (CRUR)",
    "C/T Lower Preset Value (CTLR)",
    "Mode Register B (MR1B, MR2B)",
    "Clock Select Register B (CSRB)",
    "Command Register B (CRB)",
    "Tx Holding Register B (THRB)",
    "Use for scratch pad",
    "Output Port Conf. Register (OPCR)",
    "Set Output Port Bits Command",
    "Reset Output Port Bits Command",
];

pub struct DUARTChannel {
    pub rx: mpsc::Receiver<u8>,
    pub tx: mpsc::SyncSender<u8>,
    pub dtr: Rc<Cell<bool>>,
}

impl DUARTChannel {
    pub fn new() -> (DUARTChannel, DUARTChannel) {
        let (tx, rx2) = mpsc::sync_channel(16);
        let (tx2, rx) = mpsc::sync_channel(16);
        let dtr = Rc::new(Cell::new(true));
        (
            Self {
                rx,
                tx,
                dtr: dtr.clone(),
            },
            Self {
                rx: rx2,
                tx: tx2,
                dtr,
            },
        )
    }
}

pub struct DUART {
    channel_a: DUARTChannel,
    channel_a_cooldown: u16,
    channel_b: DUARTChannel,
    channel_b_cooldown: u16,
    mode_register_a: (u8, u8),
    mr_a: Cell<bool>,
    mode_register_b: (u8, u8),
    mr_b: Cell<bool>,
    channel_a_rx_pending: Cell<Option<u8>>,
    channel_a_tx_pending: Option<u8>,
    channel_b_rx_pending: Cell<Option<u8>>,
    channel_b_tx_pending: Option<u8>,
    clock_select_warned: bool,
    reset_sleep: u16,
    interrupt_mask: u8,

    pub interrupt: bool,
    first_interrupt: bool,
    pub input_bits: u8,
    pub output_bits_inv: u8,
}

impl DUART {
    pub fn new() -> (Self, DUARTChannel, DUARTChannel) {
        let (channel_a, channel_a2) = DUARTChannel::new();
        let (channel_b, channel_b2) = DUARTChannel::new();
        (
            Self {
                channel_a,
                channel_a_cooldown: 0,
                channel_b,
                channel_b_cooldown: 0,
                mode_register_a: (0, 0),
                mode_register_b: (0, 0),
                mr_a: Cell::new(false),
                mr_b: Cell::new(false),
                channel_a_rx_pending: Cell::new(None),
                channel_a_tx_pending: None,
                channel_b_rx_pending: Cell::new(None),
                channel_b_tx_pending: None,
                input_bits: 0,
                output_bits_inv: 0,
                interrupt: false,
                interrupt_mask: 0,
                clock_select_warned: false,
                first_interrupt: true,
                reset_sleep: 0xffff,
            },
            channel_a2,
            channel_b2,
        )
    }

    pub fn read(&self, register: ReadRegister) -> u8 {
        match register {
            ReadRegister::InterruptStatusRegister => {
                let mut status = 0;
                if self.channel_a_tx_pending.is_none() {
                    status |= 0b0001;
                }
                if self.channel_a_rx_pending.get().is_some() {
                    status |= 0b0010;
                }
                if self.channel_b_tx_pending.is_none() {
                    status |= 0b0001_0000;
                }
                if self.channel_b_rx_pending.get().is_some() {
                    status |= 0b0010_0000;
                }
                status
            }
            ReadRegister::StatusRegisterA => {
                let mut status = 0;
                if self.channel_a_rx_pending.get().is_some() {
                    status |= 0b0001;
                }
                if self.channel_a_tx_pending.is_none() {
                    status |= 0b1100;
                }
                status
            }
            ReadRegister::ModeRegisterA => {
                if !self.mr_a.replace(true) {
                    trace!("DUART read MRA1");
                    self.mode_register_a.0
                } else {
                    trace!("DUART read MRA2");
                    self.mode_register_a.1
                }
            }
            ReadRegister::RxHoldingRegisterA => {
                self.channel_a_rx_pending.replace(None).take().unwrap_or(0)
            }
            ReadRegister::StatusRegisterB => {
                let mut status = 0;
                if self.channel_b_rx_pending.get().is_some() {
                    status |= 0b0001;
                }
                if self.channel_b_tx_pending.is_none() {
                    status |= 0b1100;
                }
                status
            }
            ReadRegister::ModeRegisterB => {
                if !self.mr_b.replace(true) {
                    trace!("DUART read MRB1");
                    self.mode_register_b.0
                } else {
                    trace!("DUART read MRB2");
                    self.mode_register_b.1
                }
            }
            ReadRegister::RxHoldingRegisterB => {
                self.channel_b_rx_pending.replace(None).take().unwrap_or(0)
            }
            ReadRegister::InputPortsIP0ToIP6 => self.input_bits,
            _ => {
                warn!("DUART read from unhandled register: {:?}", register);
                0
            }
        }
    }

    pub fn write(&mut self, register: WriteRegister, value: u8) {
        match register {
            WriteRegister::CommandRegisterA => match (value & 0b0111_0000) >> 4 {
                0b0001 => {
                    self.mr_a.set(false);
                }
                0b0010 => {
                    self.channel_a_rx_pending.take();
                }
                0b0011 => {
                    self.channel_a_tx_pending.take();
                }
                _ => {}
            },
            WriteRegister::ModeRegisterA => {
                if !self.mr_a.replace(true) {
                    trace!("DUART write MRA1");
                    self.mode_register_a.0 = value;
                } else {
                    trace!("DUART write MRA2");
                    self.mode_register_a.1 = value;
                }
            }
            WriteRegister::TxHoldingRegisterA => {
                self.channel_a_tx_pending = Some(value);
            }
            WriteRegister::CommandRegisterB => match (value & 0b0111_0000) >> 4 {
                0b0001 => {
                    self.mr_b.set(false);
                }
                0b0010 => {
                    self.channel_b_rx_pending.take();
                }
                0b0011 => {
                    self.channel_b_tx_pending.take();
                }
                _ => {}
            },
            WriteRegister::ModeRegisterB => {
                if !self.mr_b.replace(true) {
                    trace!("DUART write MRB1");
                    self.mode_register_b.0 = value;
                } else {
                    trace!("DUART write MRB2");
                    self.mode_register_b.1 = value;
                }
            }
            WriteRegister::SetOutputPortBitsCommand => {
                self.output_bits_inv |= value;
            }
            WriteRegister::ResetOutputPortBitsCommand => {
                self.output_bits_inv &= !value;
            }
            WriteRegister::TxHoldingRegisterB => {
                self.channel_b_tx_pending = Some(value);
            }
            WriteRegister::ClockSelectRegisterA | WriteRegister::ClockSelectRegisterB => {
                if !self.clock_select_warned {
                    warn!("DUART clock select register write ignored, running at fixed baud rate");
                    self.clock_select_warned = true;
                }
            }
            WriteRegister::InterruptMaskRegister => {
                self.interrupt_mask = value;
                if value != 0 && value != 0x22 {
                    warn!(
                        "DUART interrupt mask write only handles 0 and 0x22, other values are ignored: {value:02X}"
                    );
                }
            }
            _ => {
                warn!(
                    "DUART write of {value:02X} to to unhandled register: {:?}",
                    register
                );
            }
        }
    }

    pub fn tick(&mut self) {
        if self.reset_sleep != 0 {
            self.reset_sleep = self.reset_sleep.saturating_sub(1);
            return;
        }

        if self.mode_register_a.1 & 0b1000_0000 != 0 {
            if let Some(tx) = self.channel_a_tx_pending.take() {
                trace!(
                    "DUART pipe local loopback (channel A) {tx:02X} {:?}",
                    tx as char
                );
                self.channel_a_rx_pending.replace(Some(tx));
            }
        } else {
            if let Some(tx) = self.channel_a_tx_pending.take() {
                trace!("DUART pipe send (channel A) {tx:02X} {:?}", tx as char);
                _ = self.channel_a.tx.send(tx);
            }
            let dtr = self.channel_a.dtr.get();
            self.channel_a_cooldown = self.channel_a_cooldown.saturating_sub(1);
            if self.channel_a_rx_pending.get().is_none() && dtr && self.channel_a_cooldown == 0 {
                if let Ok(tx) = self.channel_a.rx.try_recv() {
                    trace!(
                        "DUART pipe receive (channel A, dtr = {dtr}) {tx:02X} {:?}",
                        tx as char
                    );
                    self.channel_a_rx_pending.replace(Some(tx));
                    self.channel_a_cooldown = DUART_COOLDOWN_TICKS;
                }
            }
        }
        if self.mode_register_b.1 & 0b1000_0000 != 0 {
            if let Some(tx) = self.channel_b_tx_pending.take() {
                trace!(
                    "DUART pipe local loopback (channel B) {tx:02X} {:?}",
                    tx as char
                );
                self.channel_b_rx_pending.replace(Some(tx));
            }
        } else {
            if let Some(tx) = self.channel_b_tx_pending.take() {
                trace!("DUART pipe send (channel B) {tx:02X} {:?}", tx as char);
                _ = self.channel_b.tx.send(tx);
            }
            let dtr = self.channel_b.dtr.get();
            self.channel_b_cooldown = self.channel_b_cooldown.saturating_sub(1);
            if self.channel_b_rx_pending.get().is_none() && dtr && self.channel_b_cooldown == 0 {
                if let Ok(tx) = self.channel_b.rx.try_recv() {
                    trace!(
                        "DUART pipe receive (channel B, dtr = {dtr}) {tx:02X} {:?}",
                        tx as char
                    );
                    self.channel_b_rx_pending.replace(Some(tx));
                    self.channel_b_cooldown = DUART_COOLDOWN_TICKS;
                }
            }
        }

        self.interrupt = self.interrupt_mask != 0
            && (self.channel_a_rx_pending.get().is_some()
                || self.channel_b_rx_pending.get().is_some());
        if self.interrupt && self.first_interrupt {
            warn!("First DUART interrupt fired");
            self.first_interrupt = false;
        }
    }
}
