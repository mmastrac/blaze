use std::time::Duration;

use i8051::Cpu;
use i8051_debug_tui::Debugger;

use crate::System;

pub fn run(
    mut system: System,
    mut cpu: Cpu,
    debugger: Option<Debugger>,
) -> Result<usize, Box<dyn std::error::Error>> {
    use i8051_debug_tui::{DebuggerState, crossterm};
    if let Some(mut debugger) = debugger {
        debugger.enter()?;
        loop {
            match debugger.debugger_state() {
                DebuggerState::Quit => {
                    debugger.exit()?;
                    break;
                }
                DebuggerState::Paused => {
                    debugger.render(&cpu, &mut system)?;
                    let event = crossterm::event::poll(Duration::from_millis(100))?;
                    if event {
                        let event = crossterm::event::read()?;
                        if debugger.handle_event(event, &mut cpu, &mut system) {
                            system.step(&mut cpu);
                        }
                    }
                }
                DebuggerState::Running => {
                    if system.instruction_count % 0x10000 == 0 {
                        debugger.render(&cpu, &mut system)?;
                        let event = crossterm::event::poll(Duration::from_millis(0))?;
                        if event {
                            let event = crossterm::event::read()?;
                            if debugger.handle_event(event, &mut cpu, &mut system) {
                                system.step(&mut cpu);
                                debugger.render(&cpu, &mut system)?;
                            }
                        }
                    }
                    system.step(&mut cpu);
                    if debugger.breakpoints().contains(&cpu.pc_ext(&system)) {
                        debugger.pause();
                    }
                }
            }
        }
    } else {
        loop {
            system.step(&mut cpu);
        }
    }
    Ok(system.instruction_count)
}
