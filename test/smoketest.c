/* test8051.c — SDCC 8051 test program */

#include <8051.h>  /* gives you SFR definitions like P1 */

void main(void)
{
    unsigned char i, sum = 0;
    __xdata unsigned char *scratch = (__xdata unsigned char *)0x8000;

    /* Clear scratchpad */
    for (i = 0; i < 4; i++) {
        scratch[i] = 0;
    }

    /* Simple loop: add numbers 1..10 into sum */
    for (i = 1; i <= 10; i++) {
        sum += i;
    }

    /* Store result in external memory for visibility */
    scratch[0] = sum;

    /* Expected sum of 1..10 = 55 decimal */
    if (sum == 55) {
        P1 = 0x55;   /* indicate success on Port 1 */
    } else {
        P1 = 0xAA;   /* indicate failure */
    }

    /* Stop here (endless loop) */
    while (1) {
        /* idle */
    }
}