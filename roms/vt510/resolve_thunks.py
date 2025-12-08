# -*- coding: utf-8 -*-
# 8051 Strict Thunk Auto-Detection Script (FULLY HARDWARE-CORRECT)
# Matches ONLY true:
#   SET/CLR+  -> CALL -> SET/CLR+ -> RET
#
# P1.5 / P1.6 / P1.7 map DIRECTLY to address lines:
#   P1.7 -> A16 -> 0x10000
#   P1.6 -> A17 -> 0x20000
#   P1.5 -> A18 -> 0x40000
#
# Multiple bits may be modified simultaneously.

from ghidra.program.model.symbol import SourceType
from ghidra.program.model.address import AddressSet, Address
from ghidra.program.disassemble import Disassembler
from ghidra.util.task import ConsoleTaskMonitor
from ghidra.program.model.address import AddressSet
from ghidra.program.model.symbol import RefType

task_monitor = ConsoleTaskMonitor()
mem = currentProgram.getMemory().getBlock("RAM")
listing = currentProgram.getListing()
fm = currentProgram.getFunctionManager()
dis = Disassembler.getDisassembler(currentProgram, task_monitor, None)
ref_mgr = currentProgram.getReferenceManager()

SETB  = 0xD2
CLRB  = 0xC2
LCALL = 0x12
RET   = 0x22

MAX_PATTERN_LENGTH = 4 + 3 + 4 + 1
MIN_PATTERN_LENGTH = 2 + 3 + 2 + 1

# Direct address-line mapping
BIT_TO_ADDR = {
    0x97: 0x10000,  # P1.7 -> A16
    0x96: 0x20000,  # P1.6 -> A17
    0x95: 0x40000,  # P1.5 -> A18
}

start = mem.getAddressRange().getMinAddress()
end   = mem.getAddressRange().getMaxAddress()

class Pattern:
    def __init__(self, addr, length):
        self.addr = addr
        self.length = length

    def __repr__(self):
        return "Pattern(addr={}, length={})".format(hex(self.addr), self.length)

def get_bit(addr, memory):
    if memory[0] == SETB:
        bit = BIT_TO_ADDR.get(memory[1])
        if bit is not None:
            addr = addr | bit
            return addr
    elif memory[0] == CLRB:
        bit = BIT_TO_ADDR.get(memory[1])
        if bit is not None:
            addr = addr & ~bit
            return addr
    return None

def check_pattern(addr):
    memory = []
    for i in range(MAX_PATTERN_LENGTH):
        memory.append(mem.getByte(addr.add(i)) & 0xFF)
    bank = addr.getOffset() & 0xFF0000
    if memory[0] == SETB or memory[0] == CLRB:
        if memory[2] == SETB or memory[2] == CLRB:
            if memory[4] != LCALL or memory[MAX_PATTERN_LENGTH - 1] != RET:
                return None
            # Two bits
            addr = bank | memory[5] << 8 | memory[6]
            addr = get_bit(addr, memory[0:2])
            if addr is None:
                return None
            addr = get_bit(addr, memory[2:4])
            if addr is None:
                return None
            return Pattern(addr, MAX_PATTERN_LENGTH)
        else:
            if memory[2] != LCALL or memory[MIN_PATTERN_LENGTH - 1] != RET:
                return None
            # One bit
            addr = bank | memory[3] << 8 | memory[4]
            addr = get_bit(addr, memory[0:2])
            if addr is None:
                return None
            return Pattern(addr, MIN_PATTERN_LENGTH)

    return None

def resolve_final_thunk(func):
    """
    Follow a chain of thunked functions until the final non-thunk target.
    """
    seen = set()
    while func is not None and func.isThunk():
        if func in seen:
            print("WARNING: Thunk loop detected at", func.getEntryPoint())
            break
        seen.add(func)
        func = func.getThunkedFunction(False)
    return func

addr = start

print("Checking candidates from {} to {}".format(addr, end.subtract(MAX_PATTERN_LENGTH)))
candidates = []
while addr is not None and addr < end.subtract(MAX_PATTERN_LENGTH):
    if mem.getByte(addr) & 0xFF == SETB or mem.getByte(addr) & 0xFF == CLRB:
        if mem.getByte(addr.add(MIN_PATTERN_LENGTH - 1)) & 0xFF == RET or \
            mem.getByte(addr.add(MAX_PATTERN_LENGTH - 1)) & 0xFF == RET:
            res = check_pattern(addr)
            if res:
                candidates.append((addr, res))
                addr = addr.add(res.length)
                continue
    addr = addr.next()

for candidate in candidates:
    addr, pattern = candidate
    range = AddressSet(addr, addr.add(pattern.length - 1))
    listing.clearComments(addr, addr.add(pattern.length - 1))

    for existing in fm.getFunctionsOverlapping(range):
        fm.removeFunction(existing.getEntryPoint())

    instr = listing.getInstructionAt(addr)
    if instr is None:
        print("WARNING: No instruction found at {}".format(addr))
        dis.disassemble(addr, None)
        continue

    try:
        func = fm.createFunction(None, addr, range, SourceType.USER_DEFINED)
    except Exception as e:
        print("WARNING: Failed to create function at {}: {}".format(addr, e))
        continue

for candidate in candidates:
    addr, pattern = candidate
    func = fm.getFunctionAt(addr)
    if func is None:
        print("WARNING: Function not found at {}".format(addr))
        continue
    thunk = fm.getFunctionAt(toAddr(pattern.addr))
    func.setThunkedFunction(thunk)

for candidate in candidates:
    addr, pattern = candidate
    func = fm.getFunctionAt(addr)
    final = resolve_final_thunk(func)
    if final is not None and final != func:
        try:
            func.setThunkedFunction(final)
        except Exception as e:
            print("WARNING: Failed to set thunked function at {}: {}".format(addr, e))
            continue

print("\n--- Rebuilding AUX Call-Through References (Idempotent) ---")

removed = 0
added = 0

for candidate in candidates:
    addr, pattern = candidate
    for ref in ref_mgr.getReferencesTo(toAddr(pattern.addr)):
        if (ref.getSource() == SourceType.USER_DEFINED and
            ref.getReferenceType().isCall()):
            try:
                ref_mgr.delete(ref)
                removed += 1
            except:
                pass
    for ref in ref_mgr.getReferencesTo(addr):
        if (ref.getSource() == SourceType.USER_DEFINED and
            ref.getReferenceType().isCall()):
            try:
                ref_mgr.delete(ref)
                removed += 1
            except:
                pass

for candidate in candidates:
    addr, pattern = candidate
    for ref in ref_mgr.getReferencesTo(addr):
        if (ref.getSource() != SourceType.USER_DEFINED and 
            ref.getReferenceType().isCall()):
            thunk = fm.getFunctionAt(addr).getThunkedFunction(True)
            if thunk is None:
                print("WARNING: Final thunk not found at {}".format(addr))
                continue
            ref_mgr.addMemoryReference(
                ref.getFromAddress(),
                thunk.getEntryPoint(),
                RefType.UNCONDITIONAL_CALL,
                SourceType.USER_DEFINED,
                0
            )
