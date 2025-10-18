; 8051 test case - output byte 1 to memory 0x8000

.module smoketest
.area CSEG (CODE)

_start:
    ; Set up DPTR to point to 0x8000
    mov DPTR, #0x8000

    ; Load accumulator with value 1
    mov A, #0x01

    ; Store accumulator to external memory at DPTR
    movx @DPTR, A

    ; Halt execution
halt:
    sjmp halt

.area DSEG (DATA)
.area XSEG (XDATA)
.area HOME (CODE)
.area PSEG (CODE)

