/* test8051.c — SDCC 8051 test program */

#include <8051.h>  /* gives you SFR definitions like P1 */


void main(void)
{
    __xdata unsigned char *scratch = (__xdata unsigned char *)0x8000;

    volatile int a = 10;
    volatile int b = a * 25;
    volatile int c = b / 10;
    volatile int d = (c + 1) % 10;

    /* Store result in external memory for visibility */
    scratch[0] = a & 0xff;
    scratch[1] = b & 0xff;
    scratch[2] = c & 0xff;
    scratch[3] = d & 0xff;

    /* Stop here (endless loop) */
    while (1) {
        /* idle */
    }
}
