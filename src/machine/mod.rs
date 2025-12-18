use i8051::CpuContext;

pub mod generic;
pub mod vt420;
pub mod vt510;
pub mod vt52x;

pub trait TerminalSystem: CpuContext {
    fn step(&mut self, cpu: &mut i8051::Cpu);
}

pub struct System<S: TerminalSystem> {
    pub instruction_count: usize,
    pub system: S,
}

impl<S: TerminalSystem> System<S> {
    #[inline]
    pub fn step(&mut self, cpu: &mut i8051::Cpu) {
        self.system.step(cpu);
    }

    pub fn new(system: S) -> Self {
        Self {
            instruction_count: 0,
            system,
        }
    }
}

impl<S: TerminalSystem> CpuContext for System<S> {
    type Ports = S::Ports;
    type Xdata = S::Xdata;
    type Code = S::Code;

    fn ports(&self) -> &Self::Ports {
        self.system.ports()
    }

    fn ports_mut(&mut self) -> &mut Self::Ports {
        self.system.ports_mut()
    }

    fn xdata(&self) -> &Self::Xdata {
        self.system.xdata()
    }

    fn xdata_mut(&mut self) -> &mut Self::Xdata {
        self.system.xdata_mut()
    }

    fn code(&self) -> &Self::Code {
        self.system.code()
    }

    fn code_mut(&mut self) -> &mut Self::Code {
        self.system.code_mut()
    }
}
