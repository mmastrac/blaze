use crate::host::screen::text_terminal::TextTerminal;
use crate::machine::generic::keyboard::KeyboardInput;
use crate::machine::vt52x::System as Vt52x;

impl TextTerminal for Vt52x {
    fn keyboard_input(&self) -> Box<dyn KeyboardInput> {
        Box::new(self.memory.regs.kbd.input())
    }

    fn instruction_count(&self) -> usize {
        self.instruction_count
    }

    fn redraw_interval(&self) -> usize {
        0x40000
    }
}
