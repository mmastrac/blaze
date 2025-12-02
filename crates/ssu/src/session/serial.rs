use std::{num::NonZeroU32, path::PathBuf, thread};

use serialport::{DataBits, FlowControl, StopBits};

use crate::session::IoSessionEndpoint;
use crate::session::io::IoSessionReadWrite;

pub struct SerialSession {
    path: PathBuf,
    baud_rate: NonZeroU32,
    data_bits: DataBits,
    stop_bits: StopBits,
    flow_control: Option<FlowControl>,
}

impl SerialSession {
    pub fn new(
        path: PathBuf,
        baud_rate: NonZeroU32,
        data_bits: DataBits,
        stop_bits: StopBits,
        flow_control: Option<FlowControl>,
    ) -> Self {
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
                .flow_control(self.flow_control.unwrap_or(FlowControl::None))
                .open()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
                .unwrap();
            let write = read.try_clone().unwrap();

            ready(Ok(IoSessionReadWrite::new(read, write)));
        });
    }
}
