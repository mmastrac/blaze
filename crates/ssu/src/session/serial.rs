use std::{io, num::NonZeroU32, path::PathBuf, thread, time::Duration};

use serialport::{DataBits, FlowControl, StopBits};

use crate::session::io_session::IoSession;
use crate::session::io_session::IoSessionReadWrite;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SerialFlowControl {
    Hardware,
    Software,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerialConfig {
    pub path: PathBuf,
    pub baud_rate: NonZeroU32,
    pub data_bits: u8,
    pub stop_bits: u8,
    pub flow_control: Option<SerialFlowControl>,
}

impl SerialConfig {
    fn start(self) -> io::Result<IoSessionReadWrite> {
        let data_bits = match self.data_bits {
            8 => DataBits::Eight,
            7 => DataBits::Seven,
            6 => DataBits::Six,
            5 => DataBits::Five,
            _ => unreachable!(),
        };
        let stop_bits = match self.stop_bits {
            1 => StopBits::One,
            2 => StopBits::Two,
            _ => unreachable!(),
        };
        let flow_control = match self.flow_control {
            Some(SerialFlowControl::Hardware) => FlowControl::Hardware,
            Some(SerialFlowControl::Software) => FlowControl::Software,
            None => FlowControl::None,
        };
        let read = serialport::new(self.path.to_string_lossy(), self.baud_rate.into())
            .data_bits(data_bits)
            .stop_bits(stop_bits)
            .flow_control(flow_control)
            // The default timeout returns immediately on an idle line.
            .timeout(Duration::from_millis(50))
            .open()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
            .unwrap();
        let write = read.try_clone().unwrap();
        Ok(IoSessionReadWrite::new(read, write))
    }
}

impl IoSession for SerialConfig {
    fn start(self, ready: impl FnOnce(std::io::Result<IoSessionReadWrite>) + Send + 'static) {
        thread::spawn(move || {
            let result = self.start();
            ready(result);
        });
    }
}
