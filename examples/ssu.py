#!/usr/bin/env python3
import sys, os, errno, signal, select
import datetime as dt
import traceback

LOG_PATH = "/tmp/ssu.log"

INTRO = 0x14
TERM  = 0x1C
US    = 0x1F

# Opcodes (ASCII for clarity)
OP_PROBE   = 0x21  # '!'
OP_OPEN    = 0x22  # '"'
OP_SELECT  = 0x23  # '#'
OP_ADDCR   = 0x2B  # '+'
OP_VERIFY  = 0x2D  # '-'
OP_ZERO    = 0x30  # '0'
OP_REPORT  = 0x3D  # '='
OP_DISABLE = 0x2F  # '/'
OP_RESET   = 0x2A  # '*'
OP_COLON   = 0x3A  # ':'
OP_RESTORE = 0x3C  # '<'
OP_RESTORE_END = 0x3E  # '>'
OP_REQUEST_RESTORE = 0x3B  # ';'

# Handshake frames
PROBE1      = bytes([INTRO, OP_PROBE, 0x40, 0x41, 0x42, TERM])  # 14 21 40 41 42 1C  -> !@AB
VARIANT_AAB = bytes([INTRO, OP_PROBE, 0x41, 0x41, 0x42, TERM])  # 14 21 41 41 42 1C  -> !AAB
VARIANT_BAB = bytes([INTRO, OP_PROBE, 0x42, 0x41, 0x42, TERM])  # 14 21 42 41 42 1C  -> !BAB
REPORT_OK_FOR_BANG = bytes([INTRO, OP_REPORT, OP_PROBE, 0x61, 0x40, TERM])  # 14 3D 21 61 40 1C  -> =!a@
REPORT_OK_FOR_TERM = bytes([INTRO, OP_REPORT, OP_DISABLE, 0x64, 0x40, TERM])  # 14 3D 2F 64 40 1C  -> =/a@
OPEN_UNNAMED_SESSION_A = bytes([INTRO, OP_OPEN, 0x41, US, US, TERM])  # 14 22 41 40 1C  -> " A @
OPEN_UNNAMED_SESSION_B = bytes([INTRO, OP_OPEN, 0x42, US, US, TERM])  # 14 22 42 40 1C  -> " B @
ADD_CREDITS_A = bytes([INTRO, OP_ADDCR, 0x21, 0x22, 0x40, TERM])  # 14 2B 41 40 1C  -> + A " @
ADD_CREDITS_B = bytes([INTRO, OP_ADDCR, 0x42, 0x22, 0x40, TERM])  # 14 2B 42 40 1C  -> + B " @
ZERO_CREDITS_A = bytes([INTRO, OP_ZERO, 0x41, 0x40, TERM])  # 14 30 41 40 1C  -> 0 A @
ZERO_CREDITS_B = bytes([INTRO, OP_ZERO, 0x42, 0x40, TERM])  # 14 30 42 40 1C  -> 0 B @
VERIFY_CREDITS_A = bytes([INTRO, OP_VERIFY, 0x41, TERM])  # 14 30 41 40 1C  -> - A
VERIFY_CREDITS_B = bytes([INTRO, OP_VERIFY, 0x42, TERM])  # 14 30 42 40 1C  -> - B

def printable(b): return chr(b) if 32 <= b <= 126 else '.'
def now_ms(): return dt.datetime.now().isoformat(timespec="milliseconds")

def log_per_byte(logf, prefix, data: bytes):
    logf.write(f"{now_ms()} {prefix} [{len(data)}B] HEX: {data.hex(' ')}\n")
    logf.write(f"{now_ms()} {prefix}       ASCII: {''.join(printable(x) for x in data)}\n")

def log_frame_summary(logf, fr: bytes):
    opcode = fr[1] if len(fr) >= 2 else None
    params = fr[2:-1] if len(fr) >= 3 else b""
    fields = params.split(bytes([US])) if params else []
    fields_str = " | ".join(f.hex(" ") for f in fields) if fields else "-"
    logf.write(f"{now_ms()} FRAME op=0x{opcode:02x} len={len(fr)} hex={fr.hex(' ')} fields(US-split)=[{fields_str}]\n")

def send_bytes(out, logf, data: bytes):
    try:
        out.write(data); out.flush()
        log_per_byte(logf, "OUT", data)
    except BrokenPipeError:
        logf.write(f"{now_ms()} WRITE_ERR BrokenPipe\n"); raise SystemExit(0)
    except OSError as e:
        if e.errno == errno.EPIPE:
            logf.write(f"{now_ms()} WRITE_ERR EPIPE\n"); raise SystemExit(0)
        raise

def int_to_sixbit(n: int) -> int:  # 0 -> '@'
    return 0x40 + n

def build_report(op_being_acked: int, sid_byte: int, code_int: int = 0) -> bytes:
    return bytes([INTRO, OP_REPORT, op_being_acked, sid_byte, int_to_sixbit(code_int), TERM])

def main(logf):
    # Robust stdout behavior
    try: signal.signal(signal.SIGPIPE, signal.SIG_IGN)
    except Exception: pass

    os.makedirs(os.path.dirname(LOG_PATH), exist_ok=True)
    logf.write(f"{dt.datetime.now().isoformat(timespec='seconds')} START ssu_stdio_server\n")

    infd = sys.stdin.fileno()
    out  = sys.stdout.buffer

    # --- Handshake state machine ----------------------------------------
    class HS:
        INIT, SENT_PROBE, GOT_ECHO, WAIT_VARIANT, DONE = range(5)
    hs = HS.INIT

    # Send initial probe immediately
    #send_bytes(out, logf, PROBE1)
    logf.write(f"{now_ms()} EVT sent initial probe !@AB\n")
    hs = HS.SENT_PROBE

    # --- Frame assembly vars -------------------------------------------
    in_frame = False
    frame = bytearray()

    while True:
        r, _, _ = select.select([infd], [], [], 1.0)
        if not r:
            continue

        try:
            chunk = os.read(infd, 4096)
        except OSError as e:
            if e.errno in (errno.EIO, errno.EBADF): break
            if e.errno in (errno.EAGAIN, errno.EWOULDBLOCK, errno.EINTR): continue
            logf.write(f"{now_ms()} READ_ERR {e}\n"); break

        if not chunk:
            break

        for b in chunk:
            # per-byte log
            log_per_byte(logf, "IN ", bytes([b]))

            # build frames: 0x14 ... 0x1C
            if not in_frame:
                if b == INTRO:
                    in_frame = True
                    frame.clear()
                    frame.append(b)
                else:
                    send_bytes(out, logf, bytes([b]))
                    if b == ord('x'):
                        send_bytes(out, logf, bytes([INTRO, ord('.'), ord('A'), TERM]))

            else:
                frame.append(b)
                if b == TERM:
                    fr = bytes(frame)
                    log_frame_summary(logf, fr)
                    opcode = fr[1] if len(fr) > 1 else 0x00

                    # ---------- Handshake handling ----------
                    if hs in (HS.SENT_PROBE, HS.GOT_ECHO, HS.WAIT_VARIANT):
                        if hs in (HS.SENT_PROBE, HS.WAIT_VARIANT) and (fr == VARIANT_AAB or fr == VARIANT_BAB):
                            which = "AAB" if fr == VARIANT_AAB else "BAB"
                            logf.write(f"{now_ms()} EVT got variant !{which}\n")
                            # send_bytes(out, logf, REPORT_OK_FOR_BANG)  # =!a@
                            # send_bytes(out, logf, bytes([INTRO, OP_COLON, TERM]))

                            # send_bytes(out, logf, OPEN_UNNAMED_SESSION_A)
                            # send_bytes(out, logf, bytes([INTRO, OP_SELECT, ord('A'), TERM]))
                            # send_bytes(out, logf, bytes([INTRO, OP_RESTORE, TERM]))
                            # send_bytes(out, logf, bytes([INTRO, OP_RESTORE_END, TERM]))
                            # send_bytes(out, logf, ADD_CREDITS_A)
                            # send_bytes(out, logf, bytes("Session A selected\r\n".encode('utf-8')))

                            # send_bytes(out, logf, OPEN_UNNAMED_SESSION_B)
                            # send_bytes(out, logf, bytes([INTRO, OP_SELECT, ord('B'), TERM]))
                            # send_bytes(out, logf, bytes([INTRO, OP_RESTORE, TERM]))
                            # send_bytes(out, logf, bytes([INTRO, OP_RESTORE_END, TERM]))
                            # send_bytes(out, logf, ADD_CREDITS_B)
                            # send_bytes(out, logf, bytes("Session B selected\r\n".encode('utf-8')))
                            # send_bytes(out, logf, ADD_CREDITS_B)

                            logf.write(f"{now_ms()} EVT sent =!a@ (OK) to complete handshake\n")
                            hs = HS.DONE
                        else:
                            # Not a handshake frame we care about; keep waiting for variant
                            hs = HS.WAIT_VARIANT

                    # ---------- Steady-state behavior ----------
                    reply = None
                    # if opcode == OP_PROBE:
                    #     # In practice the VT420 wants these echoed even post-handshake.
                    #     reply = fr

                    if opcode == OP_PROBE:
                        if fr == PROBE1:
                            reply = VARIANT_BAB
                        else:
                            reply = build_report(OP_PROBE, ord('a'), code_int=0)   # = ! <sid> @
                            reply += OPEN_UNNAMED_SESSION_A
                            reply += bytes([INTRO, OP_ADDCR, sid_b, 0x21, 0x22, 0x40, TERM])

                    elif opcode == OP_OPEN:
                        # params: <sid> US <label> US
                        params = fr[2:-1]
                        sid_b = params[0] if params else int_to_sixbit(0)
                        reply = build_report(OP_OPEN, sid_b, code_int=0)   # = " <sid> @
                        reply += bytes([INTRO, OP_ADDCR, sid_b, 0x21, 0x22, 0x40, TERM])

                    elif opcode == OP_SELECT:
                        params = fr[2:-1]
                        sid_b = params[0] if params else int_to_sixbit(0)
                        reply = build_report(OP_SELECT, sid_b, code_int=0) # = # <sid> @
                        reply += bytes([INTRO, OP_SELECT, sid_b, TERM])

                    elif opcode == OP_ADDCR:
                        params = fr[2:-1]
                        sid_b = params[0] if params else int_to_sixbit(0)
                        reply = None

                    elif opcode == OP_ZERO:
                        params = fr[2:-1]
                        sid_b = params[0] if params else int_to_sixbit(0)
                        reply = build_report(OP_ZERO, sid_b, code_int=0)   # = 0 <sid> @
                        reply += bytes([INTRO, OP_ADDCR, sid_b, 0x21, 0x22, 0x40, TERM])

                    elif opcode == OP_VERIFY:
                        params = fr[2:-1]
                        sid_b = params[0] if params else int_to_sixbit(0)
                        reply = build_report(OP_VERIFY, sid_b, code_int=0)   # = - <sid> @
                        reply += bytes([INTRO, OP_ADDCR, sid_b, 0x21, 0x22, 0x40, TERM])

                    elif opcode == OP_DISABLE:
                        # Reply is =/a@
                        params = fr[2:-1]
                        sid_b = params[0] if params else int_to_sixbit(0)
                        reply = build_report(OP_DISABLE, ord('a'), code_int=0)   # = / a @

                    elif opcode == OP_RESTORE:
                        reply = build_report(OP_RESTORE, ord('a'), code_int=0)   # = < a @

                    elif opcode == OP_RESTORE_END:
                        reply = build_report(OP_RESTORE_END, ord('a'),  code_int=0)   # = > a @

                    elif opcode == OP_REQUEST_RESTORE:
                        reply = build_report(OP_REQUEST_RESTORE, ord('a'), code_int=0)   # = ; a @
                        reply += bytes([INTRO, OP_RESTORE, TERM])
                        reply += bytes([INTRO, OP_RESTORE_END, TERM])

                    if reply is not None:
                        send_bytes(out, logf, reply)
                        logf.write(f"{now_ms()} EVT replied to op 0x{opcode:02x}\n")
                        if opcode == OP_DISABLE:
                            sys.exit(0)

                    in_frame = False

if __name__ == "__main__":
    with open(LOG_PATH, "a", buffering=1, encoding="utf-8") as logf:
        try:
            main(logf)
        except Exception as e:
            logf.write(f"{now_ms()} ERROR {e}\n{traceback.format_exc()}")
            sys.exit(1)
        finally:
            logf.write(f"{now_ms()} STOP ssu_stdio_server\n")