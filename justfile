ASM_BUILD_DIR := "target/asm-tests/build"
ASM_OUTPUT_DIR := "target/asm-tests"

publish:
    cargo publish -p i8051-proc-macro
    cargo publish -p i8051

make-build-dir:
    mkdir -p {{ASM_BUILD_DIR}} 2>/dev/null || true

build-asm-tests: make-build-dir
    mkdir -p "{{ASM_BUILD_DIR}}"/smoketest-0
    sdas8051 -o {{ASM_BUILD_DIR}}/smoketest-0/smoketest.rel test/smoketest.asm
    sdas8051 -l {{ASM_BUILD_DIR}}/smoketest-0/smoketest.lst test/smoketest.asm
    sdas8051 -s {{ASM_BUILD_DIR}}/smoketest-0/smoketest.sym test/smoketest.asm
    sdcc -mmcs51 --code-size 8192 -o {{ASM_BUILD_DIR}}/smoketest-0/smoketest {{ASM_BUILD_DIR}}/smoketest-0/smoketest.rel
    sdobjcopy -I ihex -O binary {{ASM_BUILD_DIR}}/smoketest-0/smoketest {{ASM_OUTPUT_DIR}}/smoketest-0.bin

    mkdir -p "{{ASM_BUILD_DIR}}"/smoketest-1
    sdcc -mmcs51 --code-size 8192 -o {{ASM_BUILD_DIR}}/smoketest-1/smoketest test/smoketest.c
    sdobjcopy -I ihex -O binary {{ASM_BUILD_DIR}}/smoketest-1/smoketest {{ASM_OUTPUT_DIR}}/smoketest-1.bin

    mkdir -p "{{ASM_BUILD_DIR}}"/math
    sdcc -mmcs51 --opt-code-speed --code-size 8192 -o {{ASM_BUILD_DIR}}/math/math test/math.c
    sdobjcopy -I ihex -O binary {{ASM_BUILD_DIR}}/math/math {{ASM_OUTPUT_DIR}}/math.bin

    mkdir -p "{{ASM_BUILD_DIR}}"/mul
    sdcc -mmcs51 --opt-code-speed --code-size 8192 -o {{ASM_BUILD_DIR}}/mul/mul test/mul.c
    sdobjcopy -I ihex -O binary {{ASM_BUILD_DIR}}/mul/mul {{ASM_OUTPUT_DIR}}/mul.bin
