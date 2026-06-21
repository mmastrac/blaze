use std::time::Duration;

use i8051::Cpu;
#[cfg(feature = "tui")]
use i8051_debug_tui::Debugger;

use crate::machine::{System, TerminalSystem};

pub fn run<S: TerminalSystem>(
    mut system: System<S>,
    mut cpu: Cpu,
    #[cfg(feature = "tui")] debugger: Option<Debugger>,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    #[cfg(feature = "tui")]
    if let Some(mut debugger) = debugger {
        use i8051_debug_tui::{DebuggerState, crossterm};
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
                            #[cfg(all(feature = "pc-trace", not(target_arch = "wasm32")))]
                            system.flush_pc_trace_if_due();
                        }
                    }
                }
                DebuggerState::Running => {
                    if system.instruction_count % 0x10000 == 0 {
                        debugger.render(&cpu, &mut system.system)?;
                        let event = crossterm::event::poll(Duration::from_millis(0))?;
                        if event {
                            let event = crossterm::event::read()?;
                            if debugger.handle_event(event, &mut cpu, &mut system) {
                                system.step(&mut cpu);
                                #[cfg(all(feature = "pc-trace", not(target_arch = "wasm32")))]
                                system.flush_pc_trace_if_due();
                                debugger.render(&cpu, &mut system)?;
                            }
                        }
                    }
                    system.step(&mut cpu);
                    #[cfg(all(feature = "pc-trace", not(target_arch = "wasm32")))]
                    system.flush_pc_trace_if_due();
                    if debugger.breakpoints().contains(&cpu.pc_ext(&system)) {
                        debugger.pause();
                    }
                }
            }
        }
        return Ok(system.instruction_count);
    }

    loop {
        system.step(&mut cpu);
        #[cfg(all(feature = "pc-trace", not(target_arch = "wasm32")))]
        system.flush_pc_trace_if_due();
    }
    Ok(system.instruction_count)
}
