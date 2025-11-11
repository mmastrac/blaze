use i8051::breakpoint::{Action, Breakpoints};
use tracing::Level;

use crate::machine::vt420::memory::ROM;

pub(crate) const BREAKPOINTS: &[(u32, &str)] = &[
    (0x0, "Interrupt: CPU reset"),
    (0x10000, "Interrupt: CPU reset"),
    (0x000B, "Interrupt: Timer0"),
    (0x10000, "Interrupt: Timer0"),
    (0x0B66, "Interrupt: Entering user code"),
    (0x0C30, "Interrupt: Leaving user code"),
    (0x10B66, "Interrupt:Entering user code"),
    (0x10C30, "Interrupt: Leaving user code"),
    (0x5A88, "Test failed!!!"),
    (0x5D5A, "Testing failed!!!"),
    (0x15ED1, "Testing keyboard serial loopback"),
    (0x16153, "Testing keyboard serial"),
    (0x0100B, "KBD: Command requires ACK"),
    (0x1100B, "KBD: Command requires ACK"),
    (0x01009, "KBD: Got ack"),
    (0x11009, "KBD: Got ack"),
    (0x15AD0, "Testing ROM Bank 1"),
    (0x020CA, "Testing ROM Bank 0"),
    (0x15AEB, "Testing phase 2"),
    (0x15B23, "RAM test"),
    (0x15B8A, "RAM test 2"),
    (0x01F51, "Test result check"),
    (0x06AD9, "Testing completed"),
    (0x0CDF2, "Testing DUART"),
    (0x02D5E, "Processing SSU probe"),
    (0x16A0D, "Dispatching keystroke"),
    (0x05521, "Loading init string"),
    (0x15BD6, "Video RAM test"),
    (0x15C11, "Video RAM test 1/even"),
    (0x15C24, "Video RAM test 1/odd"),
    (0x15C48, "Video RAM test 2/even"),
    (0x15C36, "Video RAM test 2/odd"),
    (0x15C0C, "Video RAM test failed"),
    (0x15EE4, "Video RAM checkerboard"),
    (0x15F81, "Video latch test outer"),
    (0x16074, "Video latch test"),
    (0x160BA, "Video latch test 1 failed"),
    (0x160F9, "Video latch test 2 failed"),
    (0x160C6, "Video latch test 3"),
    (0x15C0C, "Video RAM test failed"),
    (0x15C59, "Video RAM test passed"),
    (0x15CCA, "Wait for VSYNC (bank 1)"),
    (0x15CD4, "Wait for VSYNC failed (bank 1)"),
    (0x15D26, "Wait for VSYNC complete (bank 1)"),
    (0x15C89, "Check VSYNC timing"),
    (0x15CC5, "Check VSYNC timing (failed)"),
    (0x02074, "Wait for VSYNC (bank 0)"),
    (0x16153, "Keyboard test"),
    (0x16184, "Keyboard test (failed)"),
    (0x1616E, "Keyboard test (success)"),
    (0x05B6D, "NVR read"),
    (0x05B5E, "NVR read checksum"),
    (0x05B90, "NVR write"),
    (0x05C60, "NVR fail 1"),
    (0x05CBA, "NVR fail 2"),
    (0x05AB3, "NVR fail 3"),
    (0x05A59, "NVR fail 4"),
];

pub(crate) fn create_breakpoints(breakpoints: &mut Breakpoints, code: &ROM) {
    for &(addr, message) in BREAKPOINTS {
        breakpoints.add(true, addr, Action::Log(Level::INFO, message.into()));
    }

    for addr in code.find_bank_dispatch() {
        breakpoints.add(
            true,
            addr.dispatch_addr,
            Action::Log(
                Level::INFO,
                format!(
                    "Calling bank {}/{:X}h @ {:X}h",
                    addr.target_addr >> 16,
                    addr.id,
                    addr.target_addr
                )
                .into(),
            ),
        );
        breakpoints.add(
            true,
            addr.target_addr,
            Action::Log(
                Level::INFO,
                format!(
                    "Entered bank {}/{:X}h @ {:X}h",
                    addr.target_addr >> 16,
                    addr.id,
                    addr.target_addr
                )
                .into(),
            ),
        );
    }
}
