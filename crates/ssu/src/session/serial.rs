use std::{num::NonZeroU32, path::PathBuf, thread};

use serialport::{DataBits, FlowControl, StopBits};

use crate::session::io::IoSessionReadWrite;
use crate::session::{IoSessionEndpoint, SerialFlowControl};

pub struct SerialSession {
    path: PathBuf,
    baud_rate: NonZeroU32,
    data_bits: DataBits,
    stop_bits: StopBits,
    flow_control: FlowControl,
}

impl SerialSession {
    pub fn new(
        path: PathBuf,
        baud_rate: NonZeroU32,
        data_bits: u8,
        stop_bits: u8,
        flow_control: Option<SerialFlowControl>,
    ) -> Self {
        let data_bits = match data_bits {
            8 => DataBits::Eight,
            7 => DataBits::Seven,
            6 => DataBits::Six,
            5 => DataBits::Five,
            _ => unreachable!(),
        };
        let stop_bits = match stop_bits {
            1 => StopBits::One,
            2 => StopBits::Two,
            _ => unreachable!(),
        };
        let flow_control = match flow_control {
            Some(SerialFlowControl::Hardware) => FlowControl::Hardware,
            Some(SerialFlowControl::Software) => FlowControl::Software,
            None => FlowControl::None,
        };
        SerialSession {
            path,
            baud_rate,
            data_bits,
            stop_bits,
            flow_control,
        }
    }
}

impl IoSessionEndpoint for SerialSession {
    fn start(self, ready: impl FnOnce(std::io::Result<IoSessionReadWrite>) + Send + 'static) {
        thread::spawn(move || {
            let read = serialport::new(self.path.to_string_lossy(), self.baud_rate.into())
                .data_bits(self.data_bits)
                .stop_bits(self.stop_bits)
                .flow_control(self.flow_control)
                .open()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
                .unwrap();
            let write = read.try_clone().unwrap();

            ready(Ok(IoSessionReadWrite::new(read, write)));
        });
    }
}
