async function js_read() {
    await new Promise(resolve => setTimeout(resolve, 1000));
    return new TextEncoder().encode("Hello, world!\r\n");
}

function js_write(b) {
    console.log(b);
}
